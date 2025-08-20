use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::time::SystemTime;

// 消息类型枚举
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub enum MessageType {
    Join,
    Chat,
    Leave,
    PeerList,
    ConnectRequest,
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
    pub sender_token: Option<String>,
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
            sender_token: None,
        }
    }
    
    pub fn with_target(mut self, target_id: String) -> Self {
        self.target_id = Some(target_id);
        self
    }
    
    pub fn with_content(mut self, content: String) -> Self {
        self.content = Some(content);
        self
    }
    
    pub fn with_sender_address(mut self, address: String, port: u16) -> Self {
        self.sender_peer_address = address;
        self.sender_listen_port = port;
        self
    }
}

// 节点信息结构体
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PeerInfo {
    pub user_id: String,
    pub address: String,
    pub port: u16,
}

impl PeerInfo {
    pub fn new(user_id: String, address: String, port: u16) -> Self {
        PeerInfo {
            user_id,
            address,
            port,
        }
    }
    
    pub fn socket_addr(&self) -> Result<SocketAddr, std::net::AddrParseError> {
        let addr = format!("{}:{}", self.address, self.port);
        addr.parse()
    }
}

// 错误类型枚举
#[derive(Debug)]
pub enum P2PError {
    IoError(std::io::Error),
    SerializationError(serde_json::Error),
    InvalidMessage(String),
    PeerNotFound,
    ConnectionError(String),
}

impl std::fmt::Display for P2PError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            P2PError::IoError(e) => write!(f, "IO error: {}", e),
            P2PError::SerializationError(e) => write!(f, "Serialization error: {}", e),
            P2PError::InvalidMessage(s) => write!(f, "Invalid message: {}", s),
            P2PError::PeerNotFound => write!(f, "Peer not found"),
            P2PError::ConnectionError(s) => write!(f, "Connection error: {}", s),
        }
    }
}

// 常量定义
pub const SERVER_ADDR: &str = "0.0.0.0:8080";
pub const HEARTBEAT_INTERVAL: u64 = 5;

// 消息序列化和反序列化函数
pub fn serialize_message(message: &Message) -> Result<Vec<u8>, P2PError> {
    let json = serde_json::to_string(message).map_err(P2PError::SerializationError)?;
    let mut data = json.as_bytes().to_vec();
    // 添加分隔符以区分消息
    data.push(b'\n');
    Ok(data)
}

pub fn deserialize_message(data: &[u8]) -> Result<Message, P2PError> {
    serde_json::from_slice(data).map_err(P2PError::SerializationError)
}