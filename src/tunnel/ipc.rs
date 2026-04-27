#![allow(dead_code)]

use serde::{Deserialize, Serialize};

pub const MAX_RETRIES: u32 = 5;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum IpcRequest {
    Status,
    Start { forward_id: String },
    Stop { forward_id: String },
    StopAll,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum IpcResponse {
    Status(Vec<ForwardStatus>),
    Ok,
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardStatus {
    pub id: String,
    pub host_id: String,
    pub state: ForwardState,
    pub retry_count: u32,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForwardState {
    Stopped,
    Connecting,
    Running,
    Reconnecting,
    Error,
}

impl ForwardState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Connecting | Self::Running | Self::Reconnecting)
    }

    pub fn label(&self, retry_count: u32) -> String {
        match self {
            Self::Stopped => "[stopped]".into(),
            Self::Connecting => "[connecting]".into(),
            Self::Running => "[running]".into(),
            Self::Reconnecting => format!("[reconnecting {retry_count}/{}]", MAX_RETRIES),
            Self::Error => "[error]".into(),
        }
    }
}

pub fn encode_request(req: &IpcRequest) -> anyhow::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(req)?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub fn encode_response(resp: &IpcResponse) -> anyhow::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(resp)?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_request_serializes() {
        let req = IpcRequest::Status;
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"cmd\":\"status\""));
    }

    #[test]
    fn start_request_serializes() {
        let req = IpcRequest::Start {
            forward_id: "fwd-1".into(),
        };
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"start\""));
        assert!(s.contains("fwd-1"));
    }

    #[test]
    fn response_round_trip() {
        let resp = IpcResponse::Status(vec![ForwardStatus {
            id: "f1".into(),
            host_id: "h1".into(),
            state: ForwardState::Running,
            retry_count: 0,
            error: None,
        }]);
        let s = serde_json::to_string(&resp).unwrap();
        let back: IpcResponse = serde_json::from_str(&s).unwrap();
        if let IpcResponse::Status(statuses) = back {
            assert_eq!(statuses[0].state, ForwardState::Running);
        } else {
            panic!("wrong variant");
        }
    }
}
