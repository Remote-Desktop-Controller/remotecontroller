//! Read-only libgit2 adapter. No hooks, external diff or textconv processes.
use crate::files::WorkspaceFiles;
use async_trait::async_trait;
use runtime_ports::*;
use serde_json::{Value, json};
pub struct GitReader {
    files: WorkspaceFiles,
}
impl GitReader {
    pub fn new(files: WorkspaceFiles) -> Self {
        Self { files }
    }
}
#[async_trait]
impl GitPort for GitReader {
    async fn read_git(&self, command: &str, argument: Option<&str>) -> Result<Value> {
        let root = self.files.root().to_owned();
        let command = command.to_string();
        let argument = argument.map(String::from);
        tokio::task::spawn_blocking(move||{
            let git_path=root.join(".git");
            if !git_path.is_dir(){return Err(PortError::Policy("Git metadata must be a directory inside workspace".into()));}
            let mut pending=vec![git_path];let mut examined=0;
            while let Some(path)=pending.pop(){
                let m=std::fs::symlink_metadata(&path).map_err(crate::files::io_error)?;
                if m.file_type().is_symlink(){return Err(PortError::Policy("linked Git metadata denied".into()));}
                examined+=1;if examined>100_000{return Err(PortError::Policy("Git metadata entry limit".into()));}
                if m.is_dir(){for e in std::fs::read_dir(path).map_err(crate::files::io_error)?{pending.push(e.map_err(crate::files::io_error)?.path());}}
            }
            let repo=git2::Repository::open_ext(&root,git2::RepositoryOpenFlags::NO_SEARCH,std::iter::empty::<&std::path::Path>()).map_err(|e|PortError::Tool(e.to_string()))?;
            let git_error=|e:git2::Error|PortError::Tool(e.to_string());
            match command.as_str(){
                "git.status"=>{
                    let statuses=repo.statuses(None).map_err(git_error)?;
                    let entries:Vec<_>=statuses.iter().take(10_000).map(|e|Ok(json!({"path":e.path().map_err(git_error)?,"status_bits":e.status().bits()}))).collect::<Result<_>>()?;
                    Ok(json!({"entries":entries,"truncated":statuses.len()>10_000}))
                },
                "git.diff"=>{
                    let mut options=git2::DiffOptions::new();options.include_untracked(false);
                    let diff=repo.diff_index_to_workdir(None,Some(&mut options)).map_err(git_error)?;
                    let mut output=Vec::new();let mut truncated=false;
                    diff.print(git2::DiffFormat::Patch,|_,_,line|{if output.len()+line.content().len()>1024*1024{truncated=true;return true;}output.extend_from_slice(line.content());true}).map_err(git_error)?;
                    Ok(json!({"diff":String::from_utf8_lossy(&output),"truncated":truncated}))
                },
                "git.show"=>{
                    let revision=argument.ok_or_else(||PortError::Tool("revision required".into()))?;
                    if revision.len()>64||!revision.chars().all(|c|c.is_ascii_alphanumeric()||matches!(c,'_'|'-'|'/'|'.')){return Err(PortError::Policy("simple revision only".into()));}
                    let object=repo.revparse_single(&revision).map_err(git_error)?;
                    let commit=object.peel_to_commit().map_err(git_error)?;
                    Ok(json!({"commit":commit.id().to_string(),"summary":commit.summary().map_err(git_error)?,"time":commit.time().seconds(),"parents":commit.parent_ids().map(|p|p.to_string()).collect::<Vec<_>>()}))
                },_=>Err(PortError::NotFound)
            }
        }).await.map_err(|e|PortError::Tool(e.to_string()))?
    }
}
