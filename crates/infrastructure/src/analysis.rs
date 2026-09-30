use crate::files::WorkspaceFiles;
use async_trait::async_trait;
use runtime_ports::*;
use serde_json::{Value, json};

pub struct CodeAnalysis {
    files: WorkspaceFiles,
}
impl CodeAnalysis {
    pub fn new(files: WorkspaceFiles) -> Self {
        Self { files }
    }
}
#[async_trait]
impl CodeAnalysisPort for CodeAnalysis {
    async fn inspect(&self, path: &str, language: &str, query: Option<&str>) -> Result<Value> {
        let bytes = self.files.read(path).await?;
        let language = language.to_string();
        let query = query.map(String::from);
        tokio::task::spawn_blocking(move || {
            let language:tree_sitter::Language=match language.as_str(){"rust"=>tree_sitter_rust::LANGUAGE.into(),"python"=>tree_sitter_python::LANGUAGE.into(),_=>return Err(PortError::Tool("unsupported language".into()))};
            let mut parser=tree_sitter::Parser::new();parser.set_language(&language).map_err(|e|PortError::Tool(e.to_string()))?;
            let tree=parser.parse(&bytes,None).ok_or_else(||PortError::Tool("AST parse failed".into()))?;
            let root=tree.root_node();
            if let Some(query)=query {
                use tree_sitter::StreamingIterator;
                let query=tree_sitter::Query::new(&language,&query).map_err(|e|PortError::Tool(e.to_string()))?;
                let mut cursor=tree_sitter::QueryCursor::new();cursor.set_match_limit(1000);
                let mut matches=cursor.matches(&query,root,bytes.as_slice());let mut captures=Vec::new();
                while let Some(m)=matches.next(){for c in m.captures{if captures.len()>=1000{break;}captures.push(json!({"capture":query.capture_names()[c.index as usize],"kind":c.node.kind(),"start":c.node.start_byte(),"end":c.node.end_byte()}));}if captures.len()>=1000{break;}}
                Ok(json!({"has_errors":root.has_error(),"captures":captures}))
            } else {
                let mut cursor=root.walk();let symbols:Vec<_>=root.children(&mut cursor).take(1000).map(|n|json!({"kind":n.kind(),"start":n.start_byte(),"end":n.end_byte()})).collect();
                Ok(json!({"has_errors":root.has_error(),"symbols":symbols}))
            }
        }).await.map_err(|e|PortError::Tool(e.to_string()))?
    }
}
