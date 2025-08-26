use crate::common::*;
use mio::{Events, Interest, Poll, Token};
use mio::net::TcpStream;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use std::io::{Read, Write};
use std::sync::mpsc;

const SERVER: Token = Token(0);

/// 待发送的消息
#[derive(Debug, Clone)]
pub struct PendingMessage {
    pub target: MessageTarget,
    pub message: Message,
}

/// 消息目标
#[derive(Debug, Clone)]
pub enum MessageTarget {
    Server,
    Peer(Token),
}

/// 客户端控制指令
#[derive(Debug, Clone)]
pub enum ClientCommand {
    Stop,
}

pub struct P2PClient {
    poll: Poll,
    events: Events,
    server_stream: Option<TcpStream>,
    streams: HashMap<Token, TcpStream>,
    buffers: HashMap<Token, Vec<u8>>,
    user_id: String,
    server_addr: SocketAddr,
    known_peers: HashMap<String, PeerInfo>,
    // 消息发送通道
    message_sender: mpsc::Sender<PendingMessage>,
    message_receiver: mpsc::Receiver<PendingMessage>,
    // 控制指令通道
    control_sender: mpsc::Sender<ClientCommand>,
    control_receiver: mpsc::Receiver<ClientCommand>,
}

impl P2PClient {
    pub fn new(server_addr: &str, _local_port: u16, user_id: String) -> Result<Self, P2PError> {
        let server_addr: SocketAddr = server_addr.parse().map_err(|e: std::net::AddrParseError| P2PError::ConnectionError(e.to_string()))?;
        let poll = Poll::new()?;
        
        // 创建消息发送通道
        let (message_sender, message_receiver) = mpsc::channel();
        // 创建控制指令通道
        let (control_sender, control_receiver) = mpsc::channel();
        
        Ok(Self {
            poll,
            events: Events::with_capacity(1024),
            server_stream: None,
            streams: HashMap::new(),
            buffers: HashMap::new(),
            user_id,
            server_addr,
            known_peers: HashMap::new(),
            message_sender,
            message_receiver,
            control_sender,
            control_receiver,
        })
    }
    
    /// 获取消息发送器的克隆，用于在其他线程中发送消息
    pub fn get_message_sender(&self) -> mpsc::Sender<PendingMessage> {
        self.message_sender.clone()
    }
    
    /// 获取控制指令发送器，用于从外部控制客户端
    pub fn get_control_sender(&self) -> mpsc::Sender<ClientCommand> {
        self.control_sender.clone()
    }
    
    /// 便利方法：创建聊天消息（供外部使用）
    pub fn create_chat_message(&self, target_id: Option<String>, content: String) -> PendingMessage {
        let message = Message {
            msg_type: MessageType::Chat,
            sender_id: self.user_id.clone(),
            target_id,
            content: Some(content),
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
        };
        
        PendingMessage {
            target: MessageTarget::Server,
            message,
        }
    }
    
    /// 静态方法：创建聊天消息（不需要客户端实例）
    pub fn create_chat_message_static(user_id: String, target_id: Option<String>, content: String) -> PendingMessage {
        let message = Message {
            msg_type: MessageType::Chat,
            sender_id: user_id,
            target_id,
            content: Some(content),
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
        };
        
        PendingMessage {
            target: MessageTarget::Server,
            message,
        }
    }

    pub fn connect(&mut self) -> Result<(), P2PError> {
        let mut stream = TcpStream::connect(self.server_addr)?;
        self.poll.registry()
            .register(&mut stream, SERVER, Interest::READABLE | Interest::WRITABLE)?;
        
        self.server_stream = Some(stream);
        self.buffers.insert(SERVER, Vec::new());

        // 使用通道发送join消息
        let join_message = Message {
            msg_type: MessageType::Join,
            sender_id: self.user_id.clone(),
            target_id: None,
            content: None,
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
        };

        self.queue_message(MessageTarget::Server, join_message)?;
        Ok(())
    }

    /// 将消息加入发送队列（内部方法）
    fn queue_message(&self, target: MessageTarget, message: Message) -> Result<(), P2PError> {
        let pending_message = PendingMessage { target, message };
        self.message_sender.send(pending_message)
            .map_err(|_| P2PError::ConnectionError("消息发送通道已关闭".to_string()))?;
        Ok(())
    }

    /// 单次事件轮询（非阻塞）
    pub fn poll_once(&mut self) -> Result<(), P2PError> {
        self.poll.poll(&mut self.events, Some(Duration::from_millis(100)))?;
        self.process_events()
    }
    
    /// 运行客户端（纯粹的网络事件循环）
    /// 使用通道接收外部指令和消息
    pub fn run(&mut self) -> Result<(), P2PError> {
        loop {
            // 处理网络事件和待发送消息
            self.poll.poll(&mut self.events, Some(Duration::from_millis(50)))?;
            self.process_events()?;
            
            // 检查控制指令
            match self.control_receiver.try_recv() {
                Ok(ClientCommand::Stop) => {
                    println!("收到停止指令，正在关闭客户端...");
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // 没有指令，继续运行
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // 控制通道已断开，退出
                    break;
                }
            }
        }
        Ok(())
    }
    
    /// 处理网络事件（内部方法）
    fn process_events(&mut self) -> Result<(), P2PError> {
        // 先处理待发送的消息
        self.process_pending_messages()?;
        
        // 再处理网络事件
        let event_tokens: Vec<Token> = self.events.iter().map(|e| e.token()).collect();
        
        for token in event_tokens {
            match token {
                SERVER => self.handle_server_event()?,
                token => {
                    if let Some(event) = self.events.iter().find(|e| e.token() == token) {
                        if event.is_readable() {
                            self.handle_readable(token)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    
    /// 处理待发送的消息
    fn process_pending_messages(&mut self) -> Result<(), P2PError> {
        // 处理所有待发送的消息
        while let Ok(pending_message) = self.message_receiver.try_recv() {
            match pending_message.target {
                MessageTarget::Server => {
                    self.send_message_to_server(&pending_message.message)?;
                }
                MessageTarget::Peer(token) => {
                    self.send_message_to_peer(token, &pending_message.message)?;
                }
            }
        }
        Ok(())
    }

    fn handle_server_event(&mut self) -> Result<(), P2PError> {
        if let Some(stream) = &mut self.server_stream {
            let mut buffer = [0; 1024];
            match stream.read(&mut buffer) {
                Ok(0) => return Err(P2PError::ConnectionError("Server disconnected".to_string())),
                Ok(n) => {
                    if let Some(peer_buffer) = self.buffers.get_mut(&SERVER) {
                        peer_buffer.extend_from_slice(&buffer[..n]);
                    }
                    self.try_parse_messages(SERVER)?;
                }
                Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => {
                    return Err(P2PError::IoError(e));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn handle_readable(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            let mut buffer = [0; 1024];
            match stream.read(&mut buffer) {
                Ok(0) => {
                    self.remove_peer(token);
                }
                Ok(n) => {
                    if let Some(peer_buffer) = self.buffers.get_mut(&token) {
                        peer_buffer.extend_from_slice(&buffer[..n]);
                    }
                    self.try_parse_messages(token)?;
                }
                Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => {
                    self.remove_peer(token);
                    return Err(P2PError::IoError(e));
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn try_parse_messages(&mut self, token: Token) -> Result<(), P2PError> {
        let mut messages = Vec::new();
        
        if let Some(buffer) = self.buffers.get_mut(&token) {
            while let Some(delimiter_pos) = buffer.iter().position(|&b| b == b'\n') {
                let message_data = buffer.drain(..=delimiter_pos).collect::<Vec<_>>();
                let message_data = &message_data[..message_data.len() - 1];
                
                if let Ok(message) = deserialize_message(message_data) {
                    messages.push(message);
                }
            }
        }
        
        for message in messages {
            self.handle_message(&message)?;
        }
        
        Ok(())
    }

    fn handle_message(&mut self, message: &Message) -> Result<(), P2PError> {
        match message.msg_type {
            MessageType::Chat => {
                if let Some(content) = &message.content {
                    println!("[{}]: {}", message.sender_id, content);
                }
            }
            MessageType::PeerList => {
                if let Some(content) = &message.content {
                    if let Ok(peer_list) = serde_json::from_str::<Vec<(String, String, u16)>>(content) {
                        for (user_id, address, port) in peer_list {
                            if user_id != self.user_id {
                                let peer_info = PeerInfo::new(user_id, address, port);
                                self.known_peers.insert(peer_info.user_id.clone(), peer_info);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// 发送消息到服务器
    fn send_message_to_server(&mut self, message: &Message) -> Result<(), P2PError> {
        if let Some(stream) = &mut self.server_stream {
            let data = serialize_message(message)?;
            stream.write_all(&data)?;
        }
        Ok(())
    }
    
    /// 发送消息到对等节点
    fn send_message_to_peer(&mut self, token: Token, message: &Message) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            let data = serialize_message(message)?;
            stream.write_all(&data)?;
        }
        Ok(())
    }

    fn remove_peer(&mut self, token: Token) {
        self.streams.remove(&token);
        self.buffers.remove(&token);
    }

    pub fn request_peer_list(&mut self) -> Result<(), P2PError> {
        let request = Message {
            msg_type: MessageType::PeerListRequest,
            sender_id: self.user_id.clone(),
            target_id: None,
            content: None,
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
        };
        self.queue_message(MessageTarget::Server, request)
    }
}