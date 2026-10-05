use crate::{IpcError, Result, receive_limited, send};
use hmac::{Hmac, Mac};
use runtime_protocol::{Frame, HANDSHAKE_MAX_FRAME, Proof, VERSION};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncWrite};

type HmacSha256 = Hmac<Sha256>;

pub(crate) fn nonce() -> Result<[u8; 32]> {
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce)
        .map_err(|e| std::io::Error::other(format!("OS entropy unavailable: {e}")))?;
    Ok(nonce)
}

fn mac(
    secret: &[u8],
    server: bool,
    workspace: uuid::Uuid,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
) -> Result<HmacSha256> {
    if secret.len() != 32 {
        return Err(IpcError::ServerAuthentication);
    }
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(secret).map_err(|_| IpcError::ServerAuthentication)?;
    mac.update(if server {
        b"rdc-ipc/v2/server"
    } else {
        b"rdc-ipc/v2/client"
    });
    mac.update(&[0]);
    mac.update(&VERSION.to_be_bytes());
    mac.update(workspace.as_bytes());
    mac.update(client_nonce);
    mac.update(server_nonce);
    Ok(mac)
}

pub(crate) fn proof(
    secret: &[u8],
    server: bool,
    workspace: uuid::Uuid,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
) -> Result<Proof> {
    Ok(Proof(
        mac(secret, server, workspace, client_nonce, server_nonce)?
            .finalize()
            .into_bytes()
            .into(),
    ))
}

pub(crate) fn verify(
    secret: &[u8],
    server: bool,
    workspace: uuid::Uuid,
    client_nonce: &[u8; 32],
    server_nonce: &[u8; 32],
    proof: &Proof,
) -> Result<()> {
    mac(secret, server, workspace, client_nonce, server_nonce)?
        .verify_slice(&proof.0)
        .map_err(|_| IpcError::ServerAuthentication)
}

pub(crate) async fn client<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    secret: &[u8],
    expected_workspace: Option<uuid::Uuid>,
) -> Result<()> {
    let client_nonce = nonce()?;
    send(
        stream,
        &Frame::ClientHello {
            protocol_version: VERSION,
            client_nonce,
        },
    )
    .await?;
    let (workspace, server_nonce) = match receive_limited(stream, HANDSHAKE_MAX_FRAME).await? {
        Frame::ServerChallenge {
            protocol_version: VERSION,
            workspace_id,
            server_nonce,
            server_proof,
        } => {
            verify(
                secret,
                true,
                workspace_id,
                &client_nonce,
                &server_nonce,
                &server_proof,
            )?;
            if expected_workspace.is_some_and(|expected| expected != workspace_id) {
                return Err(IpcError::ServerAuthentication);
            }
            (workspace_id, server_nonce)
        }
        Frame::Error(error) => return Err(IpcError::Peer(error)),
        _ => return Err(IpcError::ServerAuthentication),
    };
    send(
        stream,
        &Frame::ClientProof {
            client_proof: proof(secret, false, workspace, &client_nonce, &server_nonce)?,
        },
    )
    .await?;
    match receive_limited(stream, HANDSHAKE_MAX_FRAME).await? {
        Frame::Welcome {
            protocol_version: VERSION,
            workspace_id,
        } if workspace_id == workspace => Ok(()),
        Frame::Error(error) => Err(IpcError::Peer(error)),
        _ => Err(IpcError::ServerAuthentication),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transcript_matches_independent_known_answer_and_roles_do_not_reflect() {
        let secret: Vec<u8> = (0..32).collect();
        let workspace = uuid::Uuid::parse_str("00112233-4455-6677-8899-aabbccddeeff").unwrap();
        let server = proof(&secret, true, workspace, &[0xa0; 32], &[0xb0; 32]).unwrap();
        let client = proof(&secret, false, workspace, &[0xa0; 32], &[0xb0; 32]).unwrap();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(
            hex(&server.0),
            "a8f18825d326822122455ddb11f5ef1f05e33b7090d48c2a3494950e920359ec"
        );
        assert_eq!(
            hex(&client.0),
            "41a8b713636b18758d0517f9b0452968a7c7576be5ab5a036634f5845ad8b3f3"
        );
        verify(&secret, true, workspace, &[0xa0; 32], &[0xb0; 32], &server).unwrap();
        assert!(verify(&secret, false, workspace, &[0xa0; 32], &[0xb0; 32], &server).is_err());
        assert!(verify(&secret, true, workspace, &[0xa1; 32], &[0xb0; 32], &server).is_err());
        assert!(
            verify(
                &secret,
                true,
                uuid::Uuid::new_v4(),
                &[0xa0; 32],
                &[0xb0; 32],
                &server
            )
            .is_err()
        );
        assert_eq!(format!("{server:?}"), "Proof(<redacted>)");
    }
}
