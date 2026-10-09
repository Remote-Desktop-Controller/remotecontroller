pub mod autostart;
mod budget;
pub mod daemon;
pub mod http;
pub mod profile;
pub mod runner;

use anyhow::{Result, ensure};
use runtime_protocol::{RequestEnvelope, ResponseEnvelope, VERSION};
use url::Url;

pub fn validate_server(server: &str, allow_http_loopback: bool) -> Result<Url> {
    let url = Url::parse(server)?;
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        "A URL não pode conter credenciais"
    );
    ensure!(
        url.query().is_none() && url.fragment().is_none() && url.path() == "/",
        "Use apenas a origem do serviço, sem path, query ou fragmento"
    );
    let loopback = match url.host() {
        Some(url::Host::Domain("localhost")) => true,
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        _ => false,
    };
    ensure!(
        url.scheme() == "https" || (allow_http_loopback && loopback && url.scheme() == "http"),
        "HTTPS obrigatório; HTTP somente em loopback com autorização explícita de teste"
    );
    Ok(url)
}

pub fn validate_reply(request: &RequestEnvelope, reply: &ResponseEnvelope) -> Result<()> {
    ensure!(
        reply.meta.protocol_version == VERSION,
        "Versão de resposta incompatível"
    );
    ensure!(
        reply.meta.operation_id == request.meta.operation_id
            && reply.meta.workspace_id == request.meta.workspace_id
            && reply.meta.request_id == request.meta.request_id,
        "Resposta IPC não pertence à requisição"
    );
    Ok(())
}

pub fn bounded_result(reply: ResponseEnvelope) -> Result<ResponseEnvelope> {
    if serde_json::to_vec(&reply)?.len() <= 700 * 1024 {
        return Ok(reply);
    }
    let operation = reply.meta.operation_id;
    Ok(ResponseEnvelope {
        meta: reply.meta,
        payload: Err(runtime_protocol::ErrorEnvelope::new(
            runtime_protocol::ErrorCode::Tool,
            format!(
                "Saída extensa preservada no computador para operation_id={operation}. Use filesystem.read_range, process.output ou process.search com janelas menores."
            ),
        )),
    })
}
