use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::{SystemTime, Instant};

// 消息来源枚举
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum MessageSource {
    Server,  // 来自服务器
    Peer,    // 来自对等节点
}

// 消息类型枚举
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum MessageType {
    Join,
    Chat,
    Leave,
    PeerList,
    PeerListRequest,
    ConnectRequest,
    ConnectResponse,
    Heartbeat,
    UserJoined,
    UserLeft
}

// 消息结构体
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Message {
    pub msg_type: MessageType,
    pub sender_id: String,
    pub target_id: Option<String>,
    pub content: Option<String>,
    pub sender_peer_address: String,
    pub sender_listen_port: u16,
    pub timestamp: SystemTime,
    #[serde(default = "default_message_source")]
    pub source: MessageSource,
}

// 默认消息来源为服务器（为了向后兼容）
fn default_message_source() -> MessageSource {
    MessageSource::Server
}

impl Message {
    pub fn new(msg_type: MessageType, sender_id: String) -> Self {
        Message {
            msg_type,
            sender_id,
            target_id: None,
            content: None,
            sender_peer_address: "".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        }
    }
    
    pub fn with_content(mut self, content: String) -> Self {
        self.content = Some(content);
        self
    }
    
    pub fn with_target(mut self, target_id: String) -> Self {
        self.target_id = Some(target_id);
        self
    }
    
    pub fn with_peer_info(mut self, address: String, port: u16) -> Self {
        self.sender_peer_address = address;
        self.sender_listen_port = port;
        self
    }
    
    pub fn with_source(mut self, source: MessageSource) -> Self {
        self.source = source;
        self
    }
}

// 节点信息结构体
#[derive(Debug, Clone)]
pub struct PeerInfo {
    pub user_id: String,
    pub address: String,
    pub port: u16,
    pub last_heartbeat: Instant,
}

impl PeerInfo {
    pub fn new(user_id: String, address: String, port: u16) -> Self {
        PeerInfo {
            user_id,
            address,
            port,
            last_heartbeat: Instant::now(),
        }
    }
    
    pub fn socket_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        format!("{}:{}", self.address, self.port).parse()
    }
}

// 错误类型枚举
#[derive(Debug)]
pub enum P2PError {
    IoError(std::io::Error),
    SerializationError(serde_json::Error),
    ConnectionError(String),
    PeerNotFound,
}

impl std::fmt::Display for P2PError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            P2PError::IoError(e) => write!(f, "IO error: {}", e),
            P2PError::SerializationError(e) => write!(f, "Serialization error: {}", e),
            P2PError::ConnectionError(s) => write!(f, "Connection error: {}", s),
            P2PError::PeerNotFound => write!(f, "Peer not found"),
        }
    }
}

impl std::error::Error for P2PError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            P2PError::IoError(e) => Some(e),
            P2PError::SerializationError(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for P2PError {
    fn from(error: std::io::Error) -> Self {
        P2PError::IoError(error)
    }
}

impl From<serde_json::Error> for P2PError {
    fn from(error: serde_json::Error) -> Self {
        P2PError::SerializationError(error)
    }
}

// 常量定义
pub const HEARTBEAT_INTERVAL: u64 = 5;

// 消息序列化和反序列化函数
pub fn serialize_message(message: &Message) -> Result<Vec<u8>, P2PError> {
    let json = serde_json::to_string(message)?;
    let mut data = json.into_bytes();
    data.push(b'\n');
    Ok(data)
}

pub fn deserialize_message(data: &[u8]) -> Result<Message, P2PError> {
    let json_str = std::str::from_utf8(data)
        .map_err(|_| P2PError::SerializationError(
            serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid UTF-8 sequence"
            ))
        ))?;
    serde_json::from_str(json_str).map_err(P2PError::SerializationError)
}
