use crate::{ApiError, ApiResult, AppState, account, pairing, security, store};
use axum::{
    extract::{Query, State},
    http::{HeaderMap, header},
    response::{Html, IntoResponse, Redirect, Response},
};
use libsql::params;
use serde::Deserialize;

pub(crate) fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=pt-BR><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>{} · Remote Desktop Controller</title><style>*{{box-sizing:border-box}}body{{font:16px system-ui,sans-serif;background:#f3f5fa;color:#18213b;margin:0}}main{{max-width:920px;margin:auto;padding:32px 20px}}header{{font-weight:700;color:#3254ce;margin-bottom:32px}}h1{{font-size:32px;line-height:1.15}}h2{{font-size:21px}}p{{line-height:1.6}}section,.card{{background:white;border:1px solid #d9dfeb;border-radius:16px;padding:24px;margin:20px 0}}label{{display:block;margin:14px 0 6px}}input{{width:100%;padding:13px;font:inherit;border:1px solid #a4afc5;border-radius:8px}}button,.button{{display:inline-block;background:#3254ce;border:0;color:white;padding:12px 18px;border-radius:8px;font:inherit;cursor:pointer;margin-top:16px;text-decoration:none}}button.secondary{{background:#e8ebf4;color:#18213b}}button:disabled{{opacity:0.6;cursor:not-allowed}}code{{overflow-wrap:anywhere;background:#edf0f8;padding:4px 7px;border-radius:4px}}small{{color:#4c5872}}a{{color:#3254ce}}.grid{{display:grid;grid-template-columns:repeat(auto-fit,minmax(250px,1fr));gap:16px}}.grid .card{{margin:0}}.badge{{display:inline-block;background:#edf0f8;border-radius:20px;padding:4px 10px;font-size:13px}}.notice{{padding:14px 18px;border-radius:10px;margin:16px 0;line-height:1.5}}.notice.success{{background:#e6f9f0;border:1px solid #72d5a3;color:#0d6838}}.notice.error{{background:#fde8e8;border:1px solid #f8b4b4;color:#9b1c1c}}footer{{margin-top:40px;color:#4c5872}}</style><main><header>Remote Desktop Controller</header>{body}<footer>Suas ferramentas são executadas no computador pareado, sob as permissões locais.</footer></main></html>",
        security::escape(title)
    )
}
pub(crate) fn form_error(error: ApiError, back: &str) -> Response {
    let message = match error.status {
        axum::http::StatusCode::UNAUTHORIZED => {
            "Não foi possível entrar. Confira seu e-mail e sua senha e tente novamente."
        }
        axum::http::StatusCode::FORBIDDEN => {
            "Este formulário expirou. Abra a página novamente e tente outra vez."
        }
        axum::http::StatusCode::TOO_MANY_REQUESTS => {
            "Muitas tentativas em pouco tempo. Aguarde um minuto e tente novamente."
        }
        _ => error.message,
    };
    (error.status,Html(page("Não foi possível continuar",&format!("<section><h1>Não foi possível continuar</h1><p>{}</p><a class=button href=\"{}\">Voltar e tentar novamente</a></section>",security::escape(message),security::escape(back))))).into_response()
}
pub(crate) fn scope_description(scope: &str) -> String {
    scope
        .split_whitespace()
        .map(|scope| match scope {
            "devices:read" => "Ver seus computadores",
            "tools:read" => "Consultar ferramentas disponíveis",
            "tools:execute" => "Solicitar operações no computador",
            _ => "Permissão não reconhecida",
        })
        .collect::<Vec<_>>()
        .join(" · ")
}
#[derive(Deserialize)]
pub(crate) struct LoginQuery {
    #[serde(default)]
    next: String,
}
fn login_page(state: &AppState, next: &str, register: bool) -> Response {
    let csrf = security::token();
    let action = if register { "/register" } else { "/login" };
    let title = if register { "Criar conta" } else { "Entrar" };
    let next = if next.starts_with("/oauth/authorize?") {
        next
    } else {
        "/"
    };
    let next_query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("next", next)
        .finish();
    let switch = if register {
        format!(
            "Já tem conta? <a href=\"/login?{}\">Entrar</a>",
            security::escape(&next_query)
        )
    } else {
        format!(
            "Primeiro acesso? <a href=\"/register?{}\">Criar conta</a>",
            security::escape(&next_query)
        )
    };
    let body = format!(
        "<section><h1>{title}</h1><p>Entre para vincular seus computadores e autorizar o acesso pelo ChatGPT.</p><form method=post action={action}><input type=hidden name=csrf value=\"{csrf}\"><input type=hidden name=next value=\"{}\"><label for=email>E-mail</label><input id=email name=email type=email autocomplete=email maxlength=254 required><label for=password>Senha</label><input id=password name=password type=password autocomplete={} minlength=12 maxlength=256 required><small>Use pelo menos 12 caracteres.</small><br><button>{title}</button></form><p>{switch}</p></section>",
        security::escape(next),
        if register {
            "new-password"
        } else {
            "current-password"
        }
    );
    let mut response = Html(page(title, &body)).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        account::cookie_header(state, "rdc_login_csrf", &csrf, 600)
            .parse()
            .unwrap(),
    );
    response
}
pub(crate) async fn login(
    State(state): State<AppState>,
    Query(input): Query<LoginQuery>,
) -> Response {
    if state.config.commercial.is_some() {
        return crate::commercial::login(&state, &input.next)
            .await
            .unwrap_or_else(IntoResponse::into_response);
    }
    login_page(&state, &input.next, false)
}
pub(crate) async fn register(
    State(state): State<AppState>,
    Query(input): Query<LoginQuery>,
) -> Response {
    if state.config.commercial.is_some() {
        return crate::commercial::login(&state, &input.next)
            .await
            .unwrap_or_else(IntoResponse::into_response);
    }
    login_page(&state, &input.next, true)
}
#[derive(Default, Deserialize)]
pub(crate) struct HomeQuery {
    #[serde(default)]
    pub paired: Option<String>,
}
pub(crate) async fn home(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HomeQuery>,
) -> ApiResult<Response> {
    let session = match account::session(&state, &headers).await {
        Ok(session) => session,
        Err(_) => return Ok(Redirect::to("/login").into_response()),
    };
    let devices = pairing::devices(&state, &session.user_id).await?;
    let mut cards = String::new();
    for device in devices {
        let id = device["device_id"].as_str().unwrap_or_default();
        let revoked = device["revoked"].as_bool().unwrap_or(false);
        let status = if revoked {
            "Revogado"
        } else if device["online"].as_bool().unwrap_or(false) {
            "Conectado"
        } else {
            "Sem conexão recente"
        };
        let revoke = if revoked {
            String::new()
        } else {
            format!(
                "<form method=post action=/devices/{id}/revoke><input type=hidden name=csrf value=\"{}\"><button class=secondary>Revogar computador</button></form>",
                session.csrf
            )
        };
        cards.push_str(&format!("<article class=card><span class=badge>{status}</span><h2>{}</h2><p><small>Identificador: <code>{id}</code></small></p>{revoke}</article>",security::escape(device["name"].as_str().unwrap_or_default())));
    }
    if cards.is_empty() {
        cards="<p>Nenhum computador pareado. Instale o agente no computador e informe abaixo o código apresentado.</p>".into();
    }
    let connections = {
        let connection = state.store.lock().await;
        let mut rows=connection.query("SELECT g.family,c.name,g.scope FROM grants g JOIN clients c ON c.id=g.client_id WHERE g.user_id=?1 AND g.revoked=0 AND g.refresh_expires_at>?2 GROUP BY g.family LIMIT 100",params![session.user_id.clone(),security::now()]).await?;
        let mut html = String::new();
        while let Some(row) = rows.next().await? {
            html.push_str(&format!("<article class=card><h2>{}</h2><p>{}</p><form method=post action=/connections/{}/revoke><input type=hidden name=csrf value=\"{}\"><button class=secondary>Revogar conexão</button></form></article>",security::escape(&store::text(&row,1)?),security::escape(&scope_description(&store::text(&row,2)?)),security::escape(&store::text(&row,0)?),session.csrf));
        }
        html
    };
    let success_banner = if query.paired.is_some() {
        "<div class=\"notice success\"><strong>Computador pareado com sucesso!</strong> Seu dispositivo está vinculado à sua conta e pronto para receber comandos autorizados.</div>"
    } else {
        ""
    };
    let body = format!(
        "<h1>Seus computadores</h1><p>Conta: {}</p>{success_banner}<div class=grid>{cards}</div><section><h2>Parear computador</h2><p>No agente instalado, escolha conectar à conta. Confira o computador e digite o código de uso único. O código expira em dez minutos.</p><form id=pair-form method=post action=/pair><input type=hidden name=csrf value=\"{}\"><label for=code>Código de pareamento</label><input id=code name=code placeholder=ABCD-EFGH autocomplete=off maxlength=12 required><button id=pair-submit>Confirmar pareamento</button></form><script>(function(){{var f=document.getElementById('pair-form');if(f){{f.addEventListener('submit',function(e){{var b=document.getElementById('pair-submit');if(b){{if(b.disabled){{e.preventDefault();return false;}}b.disabled=true;b.textContent='Confirmando pareamento...';}}}});}}}})();</script></section><section><h2>Usar pelo ChatGPT</h2><p>Quando sua conexão estiver autorizada, peça ao ChatGPT para mostrar e escolher seu computador. O agente precisa continuar aberto nesse computador.</p><p><small>A conexão pública e a publicação em catálogo ainda precisam ser validadas. Não há aplicativo publicado para instalar pelo catálogo nesta etapa.</small></p><details><summary>Conexão de teste e configuração avançada</summary><p>Em uma conta do ChatGPT com acesso a conectores personalizados, adicione este endereço nas configurações e conclua a autorização entrando nesta conta:</p><p><code>{}</code></p><p>A disponibilidade depende da conta e das opções oferecidas pela plataforma.</p></details></section><section><h2>Conexões autorizadas</h2><div class=grid>{}</div></section><form method=post action=/logout><input type=hidden name=csrf value=\"{}\"><button class=secondary>Sair da conta</button></form>",
        security::escape(&session.email),
        session.csrf,
        security::escape(&state.config.resource()),
        if connections.is_empty() {
            "<p>Nenhuma conexão autorizada.</p>"
        } else {
            &connections
        },
        session.csrf
    );
    Ok(Html(page("Seus computadores", &body)).into_response())
}
