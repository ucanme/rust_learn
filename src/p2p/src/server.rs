use crate::common::*;
use mio::{Events, Interest, Poll, Token, Waker};
use mio::net::{TcpListener, TcpStream};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use std::thread;
use std::io::{Read, Write};

// 定义服务器使用的token
const SERVER: Token = Token(0);
const WAKE: Token = Token(1);
const FIRST_PEER: Token = Token(2);

// P2P服务器结构体
pub struct P2PServer {
    addr: SocketAddr,
    listener: TcpListener,
    poll: Poll,
    events: Events,
    waker: Waker,
    streams: HashMap<Token, TcpStream>,
    buffers: HashMap<Token, Vec<u8>>,
    peers: HashMap<Token, PeerInfo>,
    user_to_token: HashMap<String, Token>,
    next_token: Token,
    last_heartbeat: Instant,
}

impl P2PServer {
    // 创建新的P2P服务器
    pub fn new(addr: &str) -> Result<Self, P2PError> {
        let addr: SocketAddr = addr.parse().map_err(|e: std::net::AddrParseError| P2PError::ConnectionError(e.to_string()))?;
        let mut listener = TcpListener::bind(addr).map_err(P2PError::IoError)?;
        let poll = Poll::new().map_err(P2PError::IoError)?;
        let waker = Waker::new(poll.registry(), WAKE).map_err(P2PError::IoError)?;
        
        // 注册服务器socket
        poll.registry()
            .register(&mut listener, SERVER, Interest::READABLE)
            .map_err(P2PError::IoError)?;
            
        Ok(Self {
            addr,
            listener,
            poll,
            events: Events::with_capacity(128),
            waker,
            streams: HashMap::new(),
            buffers: HashMap::new(),
            peers: HashMap::new(),
            user_to_token: HashMap::new(),
            next_token: FIRST_PEER,
            last_heartbeat: Instant::now(),
        })
    }
    
    // 启动服务器
    pub fn start(&mut self) -> Result<(), P2PError> {
        println!("P2P server started on {}", self.addr);
        
        loop {
            // 轮询事件
            self.poll.poll(&mut self.events, Some(Duration::from_millis(100)))
                .map_err(P2PError::IoError)?;
            
            // 收集需要处理的服务器事件
            let mut has_new_connections = false;
            
            // 收集需要处理的可读和可写事件
            let mut readable_tokens = Vec::new();
            let mut writable_tokens = Vec::new();
            
            // 收集事件信息但不立即处理
            for event in &self.events {
                println!("[DEBUG] Poll event: token={:?}, is_readable={}, is_writable={}", 
                         event.token(), event.is_readable(), event.is_writable());
                match event.token() {
                    SERVER => {
                        has_new_connections = true;
                    }
                    WAKE => {
                        // 唤醒事件，用于通知服务器有新任务
                    }
                    token => {
                        if event.is_readable() {
                            readable_tokens.push(token);
                        }
                        if event.is_writable() {
                            writable_tokens.push(token);
                        }
                    }
                }
            }
            
            // 现在处理所有收集的事件，此时events的不可变借用已经结束
            if has_new_connections {
                self.accept_new_connection()?;
            }
            
            for token in readable_tokens {
                self.handle_readable(token)?;
            }
            
            for token in writable_tokens {
                self.handle_writable(token)?;
            }
            
            // 检查心跳
            self.check_heartbeat()?;
            self.check_peer_timeouts()?;
        }
    }
    
    // 接受新连接
    fn accept_new_connection(&mut self) -> Result<(), P2PError> {
        match self.listener.accept() {
            Ok((mut stream, addr)) => {
                let token = self.next_token;
                self.next_token = Token(self.next_token.0 + 1);
                
                // 注册新连接的流
                self.poll.registry()
                    .register(&mut stream, token, Interest::READABLE)
                    .map_err(P2PError::IoError)?;
                
                self.streams.insert(token, stream);
                self.buffers.insert(token, Vec::new());
                
                println!("New client connected: {}", addr);
            },
            Err(e) => {
                if e.kind() != std::io::ErrorKind::WouldBlock {
                    return Err(P2PError::IoError(e));
                }
            }
        }
        
        Ok(())
    }
    
    // 处理可读事件
    fn handle_readable(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            let mut buffer = [0; 1024];
            match stream.read(&mut buffer) {
                Ok(0) => {
                    // 连接关闭
                    self.remove_peer(token);
                    Ok(())
                }
                Ok(n) => {
                    // 将数据添加到缓冲区
                    let buffer = &buffer[..n];
                    if let Some(peer_buffer) = self.buffers.get_mut(&token) {
                        peer_buffer.extend_from_slice(buffer);
                    }
                    
                    // 尝试解析消息
                    self.try_parse_messages(token)?;
                    Ok(())
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        // 暂时无法读取，继续
                        Ok(())
                    } else {
                        // 发生错误，处理断开连接
                        self.remove_peer(token);
                        Err(P2PError::IoError(e))
                    }
                }
            }
        } else {
            Ok(())
        }
    }
    
    // 尝试解析缓冲区中的消息
    fn try_parse_messages(&mut self, token: Token) -> Result<(), P2PError> {
        // 先收集所有完整的消息
        let messages = {
            let buffer = self.buffers.get_mut(&token).unwrap();
            let mut messages = Vec::new();
            
            // 寻找消息分隔符
            while let Some(delimiter_pos) = buffer.iter().position(|&b| b == b'\n') {
                let message_data = buffer.drain(..=delimiter_pos).collect::<Vec<_>>();
                let message_data = &message_data[..message_data.len() - 1]; // 移除分隔符
                
                // 解析消息
                if let Ok(message) = deserialize_message(message_data) {
                    messages.push(message);
                } else {
                    println!("Failed to parse message from peer {:?}", token);
                }
            }
            
            messages
        };
        
        // 现在处理所有解析出的消息，此时对buffer的可变借用已经结束
        for message in messages {
            self.handle_message(&message, token)?;
        }
        
        Ok(())
    }
    
    // 处理消息
    fn handle_message(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
    
        match message.msg_type {
            MessageType::Join => self.handle_join_message(message, token)?,
            MessageType::Leave => self.handle_leave_message(message, token)?,
            MessageType::Chat => self.handle_chat_message(message)?,
            MessageType::Heartbeat => self.handle_heartbeat_message(token)?,
            MessageType::PeerListRequest => self.handle_peer_list_request(token)?,
            MessageType::ConnectRequest => self.handle_connect_request(message, token)?,
            _ =>{ println!("Unknown message type: {:?}", message.msg_type);
        }
        }
        
        Ok(())
    }
    
    // 处理加入消息
    fn handle_join_message(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
        let user_id = &message.sender_id;
        
        // 创建节点信息
        let peer_info = PeerInfo::new(
            user_id.clone(),
            message.sender_peer_address.clone(),
            message.sender_listen_port
        );
        
        // 存储节点信息
        self.peers.insert(token, peer_info);
        self.user_to_token.insert(user_id.clone(), token);
        
        println!("User {} joined", user_id);
        
        // 创建加入通知消息
        let join_notification = Message {
            msg_type: MessageType::UserJoined,  // 使用UserJoined而不是Join类型
            sender_id: user_id.clone(),
            target_id: None,
            content: Some(user_id.clone()),  // 将用户ID放入content字段，以便客户端能够正确处理
            sender_peer_address: message.sender_peer_address.clone(),
            sender_listen_port: message.sender_listen_port,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        // 广播加入通知给其他所有用户
        let recipients: Vec<_> = self.peers.keys()
            .filter(|&t| *t != token)
            .cloned()
            .collect();
        
        for peer_token in recipients {
            self.send_message(peer_token, &join_notification)?;
        }
        
        // 发送已知用户列表给新加入的用户
        self.send_peer_list(token)?;
        
        Ok(())
    }
    
    // 处理离开消息
    fn handle_leave_message(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
        let user_id = &message.sender_id;
        
        // 移除用户信息
        self.remove_peer(token);
        
        println!("User {} left", user_id);
        
        // 创建离开通知消息
        let leave_notification = Message {
            msg_type: MessageType::UserLeft,  // 使用UserLeft而不是Leave类型
            sender_id: user_id.clone(),
            target_id: None,
            content: Some(user_id.clone()),  // 将用户ID放入content字段，以便客户端能够正确处理
            sender_peer_address: String::new(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        // 广播离开通知给其他所有用户
        let recipients: Vec<_> = self.peers.keys().cloned().collect();
        
        for peer_token in recipients {
            self.send_message(peer_token, &leave_notification)?;
        }
        
        Ok(())
    }
    
    // 处理聊天消息
    fn handle_chat_message(&mut self, message: &Message) -> Result<(), P2PError> {
        println!("[DEBUG] Server handling chat message: from={}, target={:?}", 
                 message.sender_id, message.target_id);
        
        if let Some(target_id) = &message.target_id {
            // 私聊消息
            if let Some(token) = self.user_to_token.get(target_id) {
                println!("[DEBUG] Sending private message to {} (token={:?})", target_id, token);
                self.send_message(*token, message)?;
            } else {
                println!("Target user {} not found", target_id);
            }
        } else {
            // 广播消息
            let recipients: Vec<_> = self.peers.keys().cloned().collect();
            
            println!("[DEBUG] Broadcasting message to {} recipients", recipients.len());
            for token in recipients {
                self.send_message(token, message)?;
            }
        }
        
        Ok(())
    }
    
    // 处理心跳消息
    fn handle_heartbeat_message(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(peer_info) = self.peers.get_mut(&token) {
            peer_info.last_heartbeat = Instant::now();
        }
        
        Ok(())
    }
    
    // 处理节点列表请求
    fn handle_peer_list_request(&mut self, token: Token) -> Result<(), P2PError> {
        println!("[DEBUG] Handling peer list request from token: {:?}", token);
        self.send_peer_list(token)?;
        Ok(())
    }
    
    // 处理连接请求
    fn handle_connect_request(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
        if let Some(target_id) = &message.target_id {
            if let Some(target_token) = self.user_to_token.get(target_id) {
                if let Some(peer_info) = self.peers.get(target_token) {
                    // 创建连接响应消息，在content中包含地址和端口信息
                    let content = format!("{},{}", peer_info.address, peer_info.port);
                    let connect_response = Message {
                        msg_type: MessageType::ConnectResponse,
                        sender_id: peer_info.user_id.clone(),
                        target_id: Some(message.sender_id.clone()),
                        content: Some(content),
                        sender_peer_address: peer_info.address.clone(),
                        sender_listen_port: peer_info.port,
                        timestamp: SystemTime::now(),
                        sender_token: None,
                    };
                    
                    self.send_message(token, &connect_response)?;
                }
            }
        }
        
        Ok(())
    }
    
    // 处理可写事件
    fn handle_writable(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            if let Some(buffer) = self.buffers.get_mut(&token) {
                if !buffer.is_empty() {
                    match stream.write(buffer.as_slice()) {
                        Ok(n) => {
                            if n < buffer.len() {
                                // 部分数据已发送，保留剩余部分
                                buffer.drain(0..n);
                                // 重新注册可写事件
                                self.poll.registry()
                                    .reregister(stream, token, Interest::READABLE | Interest::WRITABLE)
                                    .map_err(P2PError::IoError)?;
                            } else {
                                // 所有数据已发送，清空缓冲区
                                buffer.clear();
                                // 只注册可读事件
                                self.poll.registry()
                                    .reregister(stream, token, Interest::READABLE)
                                    .map_err(P2PError::IoError)?;
                            }
                        }
                        Err(e) => {
                            if e.kind() == std::io::ErrorKind::WouldBlock {
                                // 暂时无法写入，继续注册可写事件
                                self.poll.registry()
                                    .reregister(stream, token, Interest::READABLE | Interest::WRITABLE)
                                    .map_err(P2PError::IoError)?;
                            } else {
                                // 发生错误，处理断开连接
                                self.remove_peer(token);
                                return Err(P2PError::IoError(e));
                            }
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    // 发送消息
    fn send_message(&mut self, token: Token, message: &Message) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            let data = serialize_message(message)?;
            
            match stream.write(&data) {
                Ok(n) => {
                    if n < data.len() {
                        // 部分数据已发送，将剩余部分存入缓冲区
                        if let Some(buffer) = self.buffers.get_mut(&token) {
                            buffer.extend_from_slice(&data[n..]);
                            // 注册可写事件
                            self.poll.registry()
                                .reregister(stream, token, Interest::READABLE | Interest::WRITABLE)
                                .map_err(P2PError::IoError)?;
                        }
                    }
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::WouldBlock {
                        // 暂时无法写入，将数据存入缓冲区
                        if let Some(buffer) = self.buffers.get_mut(&token) {
                            buffer.extend_from_slice(&data);
                            // 注册可写事件
                            self.poll.registry()
                                .reregister(stream, token, Interest::READABLE | Interest::WRITABLE)
                                .map_err(P2PError::IoError)?;
                        }
                    } else {
                        // 发生错误，处理断开连接
                        self.remove_peer(token);
                        return Err(P2PError::IoError(e));
                    }
                }
            }
        }
        
        Ok(())
    }
    
    // 移除节点
    fn remove_peer(&mut self, token: Token) {
        // 从所有映射中移除节点信息
        if let Some(peer_info) = self.peers.remove(&token) {
            self.user_to_token.remove(&peer_info.user_id);
        }
        self.streams.remove(&token);
        self.buffers.remove(&token);
        
        println!("Removed peer: {:?}", token);
    }
    
    // 发送节点列表
    fn send_peer_list(&mut self, token: Token) -> Result<(), P2PError> {
        // 收集所有节点信息
        let peer_list: Vec<_> = self.peers.values()
            .map(|info| (info.user_id.clone(), info.address.clone(), info.port))
            .collect();
        
        // 序列化节点列表
        let peer_list_data = serde_json::to_vec(&peer_list).map_err(P2PError::SerializationError)?;
        
        // 创建节点列表消息
        let peer_list_message = Message {
            msg_type: MessageType::PeerList,
            sender_id: "SERVER".to_string(),
            target_id: None,
            content: Some(String::from_utf8_lossy(&peer_list_data).to_string()),
            sender_peer_address: String::new(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        println!("[DEBUG] Sending peer list to token: {:?}", token);
        self.send_message(token, &peer_list_message)?;
        
        Ok(())
    }
    
    // 检查心跳
    fn check_heartbeat(&mut self) -> Result<(), P2PError> {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat) > Duration::from_secs(30) {
            self.broadcast_heartbeat()?;
            self.last_heartbeat = now;
        }
        
        Ok(())
    }
    
    // 广播心跳
    fn broadcast_heartbeat(&mut self) -> Result<(), P2PError> {
        let heartbeat_message = Message {
            msg_type: MessageType::Heartbeat,
            sender_id: "SERVER".to_string(),
            target_id: None,
            content: None,
            sender_peer_address: String::new(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        // 广播心跳给所有节点
        let recipients: Vec<_> = self.peers.keys().cloned().collect();
        
        for token in recipients {
            self.send_message(token, &heartbeat_message)?;
        }
        
        Ok(())
    }
    
    // 检查节点超时
    fn check_peer_timeouts(&mut self) -> Result<(), P2PError> {
        let now = Instant::now();
        let timeout_duration = Duration::from_secs(60); // 60秒超时
        
        // 找出超时的节点
        let timeout_tokens: Vec<_> = self.peers.iter()
            .filter(|(_, info)| now.duration_since(info.last_heartbeat) > timeout_duration)
            .map(|(token, _)| *token)
            .collect();
        
        // 移除超时的节点
        for token in timeout_tokens {
            self.remove_peer(token);
        }
        
        Ok(())
    }
}