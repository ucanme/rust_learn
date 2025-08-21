use crate::common::*;
use mio::{Events, Interest, Poll, Token, Waker};
use mio::net::{TcpListener, TcpStream};
use std::collections::HashMap;
use std::net::{SocketAddr, IpAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use std::thread;
use std::io::{Read, Write};

// 定义客户端使用的token
const SERVER: Token = Token(0);
const WAKE: Token = Token(1);
const FIRST_PEER: Token = Token(2);
const LOCAL_LISTENER: Token = Token(100);

// P2P客户端结构体
pub struct P2PClient {
    server_addr: SocketAddr,
    local_listen_port: u16,
    user_id: String,
    poll: Poll,
    events: Events,
    waker: Waker,
    server_stream: Option<TcpStream>,
    local_listener: Option<TcpListener>,
    streams: HashMap<Token, TcpStream>,
    buffers: HashMap<Token, Vec<u8>>,
    connected_peers: HashMap<String, Token>, // 用户ID -> Token
    known_peers: HashMap<String, PeerInfo>, // 用户ID -> 节点信息
    next_token: Token,
    last_heartbeat: Instant,
    has_sent_join_message: bool,
}

impl P2PClient {
    // 创建新的P2P客户端
    // 创建新的P2P客户端
    pub fn new(server_addr: &str, local_listen_port: u16, user_id: String) -> Result<Self, P2PError> {
        let server_addr: SocketAddr = server_addr.parse().map_err(|e: std::net::AddrParseError| P2PError::ConnectionError(e.to_string()))?;
        let poll = Poll::new().map_err(P2PError::IoError)?;
        let waker = Waker::new(poll.registry(), WAKE).map_err(P2PError::IoError)?;
        
        Ok(Self {
            server_addr,
            local_listen_port,
            user_id,
            poll,
            events: Events::with_capacity(128),
            waker,
            server_stream: None,
            local_listener: None,
            streams: HashMap::new(),
            buffers: HashMap::new(),
            connected_peers: HashMap::new(),
            known_peers: HashMap::new(),
            next_token: FIRST_PEER,
            last_heartbeat: Instant::now(),
            has_sent_join_message: false,
        })
    }
    
    // 启动客户端并初始化 - 公共API
    pub fn start(&mut self) -> Result<(), P2PError> {
        // 启动本地监听服务
        self.start_local_listener()?;
        
        // 连接到服务器
        self.connect_to_server()?;
        
        println!("P2P client started, user ID: {}", self.user_id);
        
        // 发送加入消息
        self.send_join_message()?;
        
        // 主事件循环 - 这里保持原始行为以兼容现有代码
        loop {
            // 轮询事件
            self.poll.poll(&mut self.events, Some(Duration::from_millis(100)))
                .map_err(P2PError::IoError)?;
            
            // 处理所有事件
            // 先收集所有事件信息，避免在循环中同时借用self.events和修改self
            let events_to_process: Vec<(Token, bool, bool)> = self.events
                .iter()
                .map(|e| (e.token(), e.is_readable(), e.is_writable()))
                .collect();
            
            for (token, is_readable, is_writable) in events_to_process {
                match token {
                    SERVER => {
                        // 处理服务器连接的事件
                        if is_readable {
                            self.handle_readable(SERVER)?;
                        }
                        if is_writable {
                            self.handle_writable(SERVER)?;
                        }
                    }
                    WAKE => {
                        // 唤醒事件
                    }
                    LOCAL_LISTENER => {
                        // 处理本地监听的新连接
                        self.accept_peer_connection()?;
                    }
                    token => {
                        // 处理P2P连接的事件
                        if is_readable {
                            self.handle_readable(token)?;
                        }
                        if is_writable {
                            self.handle_writable(token)?;
                        }
                    }
                }
            }
            
            // 检查心跳
            self.check_heartbeat()?;
        }
    }
    
    // 初始化客户端但不启动事件循环 - 用于锁外运行场景
    pub fn initialize(&mut self) -> Result<(), P2PError> {
        // 确保客户端尚未初始化
        if self.server_stream.is_some() {
            return Ok(()); // 已经初始化，直接返回成功
        }
        
        // 启动本地监听服务
        self.start_local_listener()?;
        
        // 连接到服务器
        self.connect_to_server()?;
        
        println!("P2P client initialized, user ID: {}", self.user_id);
        
        // 发送加入消息
        self.send_join_message()?;
        
        return Ok(());
    }
    
    // 检查是否已经初始化（服务器连接已建立）
    pub fn is_initialized(&self) -> bool {
        self.server_stream.is_some()
    }
    
    // 执行单次事件循环的逻辑 - 公共API
    pub fn poll_once(&mut self) -> Result<(), P2PError> {
        // 确保客户端已经初始化
        if self.server_stream.is_none() {
            return Err(P2PError::ConnectionError("Client not initialized".to_string()));
        }
        
        // 轮询事件
        self.poll.poll(&mut self.events, Some(Duration::from_millis(10)))
            .map_err(P2PError::IoError)?;
        
        // 处理所有事件
        // 先收集所有事件信息，避免在循环中同时借用self.events和修改self
        let events_to_process: Vec<(Token, bool, bool)> = self.events
            .iter()
            .map(|e| (e.token(), e.is_readable(), e.is_writable()))
            .collect();
        
        for (token, is_readable, is_writable) in events_to_process {
            match token {
                SERVER => {
                    // 处理服务器连接的事件
                    if is_readable {
                        self.handle_readable(SERVER)?;
                    }
                    if is_writable {
                        self.handle_writable(SERVER)?;
                    }
                }
                WAKE => {
                    // 唤醒事件
                }
                LOCAL_LISTENER => {
                    // 处理本地监听的新连接
                    self.accept_peer_connection()?;
                }
                token => {
                    // 处理P2P连接的事件
                    if is_readable {
                        self.handle_readable(token)?;
                    }
                    if is_writable {
                        self.handle_writable(token)?;
                    }
                }
            }
        }
        
        // 检查心跳
        self.check_heartbeat()?;
        
        Ok(())
    }
    
    // 启动本地监听服务
    fn start_local_listener(&mut self) -> Result<(), P2PError> {
        // 尝试绑定到指定端口或让系统分配可用端口
        let mut listener = match TcpListener::bind(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), self.local_listen_port)) {
            Ok(listener) => listener,
            Err(e) => {
                // 如果指定了端口且被占用，尝试使用0让系统自动分配
                if self.local_listen_port != 0 && e.kind() == std::io::ErrorKind::AddrInUse {
                    println!("[DEBUG] Port {} already in use, trying auto-allocation...", self.local_listen_port);
                    TcpListener::bind(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), 0))
                        .map_err(|err| P2PError::IoError(err))?
                } else {
                    return Err(if e.kind() == std::io::ErrorKind::AddrInUse {
                        P2PError::ConnectionError(format!("Port {} already in use. Try another port or use 0 for auto-allocation.", self.local_listen_port))
                    } else {
                        P2PError::IoError(e)
                    });
                }
            }
        };
        
        // 获取实际绑定的端口
        let actual_addr = listener.local_addr().map_err(P2PError::IoError)?;
        self.local_listen_port = actual_addr.port();
        
        // 注册本地监听到poll
        self.poll.registry()
            .register(&mut listener, LOCAL_LISTENER, Interest::READABLE)
            .map_err(P2PError::IoError)?;
        
        self.local_listener = Some(listener);
        
        println!("Local listener started on port {}", self.local_listen_port);
        
        Ok(())
    }
    
    // 连接到服务器
    fn connect_to_server(&mut self) -> Result<(), P2PError> {
        // 创建一个非阻塞的TCP流连接到服务器
        let mut stream = TcpStream::connect(self.server_addr).map_err(P2PError::IoError)?;
        
        // 注册服务器连接到poll
        self.poll.registry()
            .register(&mut stream, SERVER, Interest::READABLE | Interest::WRITABLE)
            .map_err(P2PError::IoError)?;
        
        self.server_stream = Some(stream);
        self.buffers.insert(SERVER, Vec::new());
        
        println!("Connecting to server at {}", self.server_addr);
        
        Ok(())
    }
    
    // 发送加入消息
    fn send_join_message(&mut self) -> Result<(), P2PError> {
        let join_message = Message {
            msg_type: MessageType::Join,
            sender_id: self.user_id.clone(),
            target_id: None,
            content: None,
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: self.local_listen_port,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        self.send_message(SERVER, &join_message)?;
        
        Ok(())
    }
    
    // 接受P2P连接
    fn accept_peer_connection(&mut self) -> Result<(), P2PError> {
        if let Some(listener) = self.local_listener.as_mut() {
            match listener.accept() {
                Ok((mut stream, addr)) => {
                    let token = self.next_token;
                    self.next_token = Token(self.next_token.0 + 1);
                    
                    // 注册新连接的流
                    self.poll.registry()
                        .register(&mut stream, token, Interest::READABLE)
                        .map_err(P2PError::IoError)?;
                    
                    self.streams.insert(token, stream);
                    self.buffers.insert(token, Vec::new());
                    
                    println!("New peer connected: {}", addr);
                },
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::WouldBlock {
                        return Err(P2PError::IoError(e));
                    }
                }
            }
        }
        
        Ok(())
    }
    
    // 处理可读事件
    fn handle_readable(&mut self, token: Token) -> Result<(), P2PError> {
        let stream = if token == SERVER {
            self.server_stream.as_mut().ok_or(P2PError::PeerNotFound)?
        } else {
            self.streams.get_mut(&token).ok_or(P2PError::PeerNotFound)?
        };
        
        let mut buffer = [0; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => {
                // 连接关闭
                self.handle_disconnection(token)?;
                Ok(())
            }
            Ok(n) => {
                let data = &buffer[..n];
                self.buffers.get_mut(&token).unwrap().extend_from_slice(data);
                
                // 尝试解析消息
                self.try_parse_messages(token)?;
                Ok(())
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::WouldBlock {
                    Ok(())
                } else {
                    // 发生错误，处理断开连接
                    self.handle_disconnection(token)?;
                    Err(P2PError::IoError(e))
                }
            }
        }
    }
    
    // 尝试解析缓冲区中的消息
    fn try_parse_messages(&mut self, token: Token) -> Result<(), P2PError> {
        let mut parsed_messages = Vec::new();
        
        {
            let buffer = self.buffers.get_mut(&token).unwrap();
            
            // 寻找消息分隔符并解析消息，但先不处理
            while let Some(delimiter_pos) = buffer.iter().position(|&b| b == b'\n') {
                let message_data = buffer.drain(..=delimiter_pos).collect::<Vec<_>>();
                let message_data = &message_data[..message_data.len() - 1]; // 移除分隔符
                
                // 解析消息并存储
                if let Ok(message) = deserialize_message(message_data) {
                    parsed_messages.push(message);
                } else {
                    println!("Failed to parse message");
                }
            }
        }
        
        // 释放对buffer的借用后再处理消息
        for message in parsed_messages {
            self.handle_message(&message, token)?;
        }
        
        Ok(())
    }
    
    // 处理消息
    fn handle_message(&mut self, message: &Message, _token: Token) -> Result<(), P2PError> {
        match message.msg_type {
            MessageType::UserJoined => {
                println!("[DEBUG] Received UserJoined message");
                if let Some(user_id) = &message.content {
                    println!("User joined1: {}", user_id);
                    // 请求节点列表以获取新用户的详细信息
                    self.request_peer_list()?;
                     println!("User joined2: {}", user_id);
                }
            }
            MessageType::UserLeft => {
                if let Some(user_id) = &message.content {
                    println!("User left: {}", user_id);
                    // 移除已知节点信息
                    self.known_peers.remove(user_id);
                    // 关闭与该用户的P2P连接（如果存在）
                    if let Some(peer_token) = self.connected_peers.remove(user_id) {
                        self.streams.remove(&peer_token);
                        self.buffers.remove(&peer_token);
                    }
                }
            }
            MessageType::PeerList => {
                if let Some(peer_list_json) = &message.content {
                    // 解析节点列表
                    let peer_list: Vec<PeerInfo> = serde_json::from_str(peer_list_json)
                        .map_err(P2PError::SerializationError)?;
                    
                    // 更新已知节点信息
                    for peer in peer_list {
                        self.known_peers.insert(peer.user_id.clone(), peer);
                    }
                    
                    println!("Updated peer list, total {} peers", self.known_peers.len());
                }
            }
            MessageType::ConnectRequest => {
                if let Some(connect_info) = &message.content {
                    // 处理连接请求信息
                    let parts: Vec<&str> = connect_info.split(',').collect();
                    if parts.len() == 2 {
                        let address = parts[0].to_string();
                        if let Ok(port) = parts[1].parse::<u16>() {
                            // 这里可以选择自动连接到其他节点或提示用户
                            println!("Received connection info for peer {}: {}:{}", 
                                     message.sender_id, address, port);
                        }
                    }
                }
            }
            MessageType::ConnectResponse => {
                // 处理连接响应消息，这是建立P2P连接的关键
                println!("[DEBUG] Received ConnectResponse from server for peer {}", message.sender_id);
                if let Some(connect_info) = &message.content {
                    let parts: Vec<&str> = connect_info.split(',').collect();
                    if parts.len() == 2 {
                        let address = parts[0].to_string();
                        if let Ok(port) = parts[1].parse::<u16>() {
                            // 更新已知节点信息
                            let peer_info = PeerInfo::new(
                                message.sender_id.clone(),
                                address.clone(),
                                port
                            );
                            self.known_peers.insert(message.sender_id.clone(), peer_info);
                            println!("[DEBUG] Updated peer info for {}: {}:{}", message.sender_id, address, port);
                        }
                    }
                }
                // 尝试连接到目标用户
                if let Err(e) = self.connect_to_peer(message.sender_id.clone()) {
                    eprintln!("[DEBUG] Failed to connect to peer {}: {}", message.sender_id, e);
                } else {
                    println!("[DEBUG] Successfully initiated connection to peer {}", message.sender_id);
                }
            }
            MessageType::Chat => {
                println!("[DEBUG] Received chat message: type={:?}, sender={}, target={:?}", 
                         message.msg_type, message.sender_id, message.target_id);
                if let Some(content) = &message.content {
                    println!("[DEBUG] Chat message content: '{}'", content);
                    println!("[{}] {}", message.sender_id, content);
                } else {
                    println!("[DEBUG] Received chat message with empty content from {}", message.sender_id);
                }
            }
            MessageType::Heartbeat => {
                // 收到心跳，无需特殊处理
            }
            _ => {
                // 忽略不支持的消息类型
                println!("Received unsupported message type: {:?}", message.msg_type);
            }
        }
        
        Ok(())
    }
    
    // 处理可写事件
    fn handle_writable(&mut self, token: Token) -> Result<(), P2PError> {
        let stream = if token == SERVER {
            self.server_stream.as_mut().ok_or(P2PError::PeerNotFound)?
        } else {
            self.streams.get_mut(&token).ok_or(P2PError::PeerNotFound)?
        };
        
        // 对于服务器连接，如果尚未发送加入消息，则发送
        if token == SERVER && !self.has_sent_join_message {
            // 检查连接是否已完全建立
            match stream.peer_addr() {
                Ok(_) => {
                    // 连接已完全建立，发送加入消息
                    self.send_join_message()?;
                    self.has_sent_join_message = true;
                    println!("Successfully connected to server at {}", self.server_addr);
                },
                Err(e) => {
                    if e.kind() != std::io::ErrorKind::NotConnected {
                        self.handle_disconnection(token)?;
                        return Err(P2PError::ConnectionError(e.to_string()));
                    }
                    // 连接尚未建立，继续等待
                }
            }
            
            // 即使连接未建立，也不要处理缓冲区，继续返回
            return Ok(());
        }
        
        if let Some(buffer) = self.buffers.get_mut(&token) {
            if !buffer.is_empty() {
                match stream.write(&buffer) {
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
                            self.handle_disconnection(token)?;
                            return Err(P2PError::IoError(e));
                        }
                    }
                }
            }
        }
        
        Ok(())
    }
    
    // 发送消息
    fn send_message(&mut self, token: Token, message: &Message) -> Result<(), P2PError> {

        let stream = if token == SERVER {
            self.server_stream.as_mut().ok_or(P2PError::PeerNotFound)?
        } else {
            println!("[DEBUG] Sending message to peer {:?}", message);
            self.streams.get_mut(&token).ok_or(P2PError::PeerNotFound)?
        };

        // 检查流是否仍然连接
        if let Some(err) = stream.take_error().unwrap_or(None) {
            self.handle_disconnection(token)?;
            return Err(P2PError::ConnectionError(err.to_string()));
        }

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
                    self.handle_disconnection(token)?;
                    // 检查是否是连接错误
                    if e.raw_os_error() == Some(57) { // 57 是 "Socket is not connected" 错误码
                        return Err(P2PError::ConnectionError("Socket is not connected".to_string()));
                    }
                    return Err(P2PError::IoError(e));
                }
            }
        }
        
        Ok(())
    }
    
    // 处理断开连接
    fn handle_disconnection(&mut self, token: Token) -> Result<(), P2PError> {
        if token == SERVER {
            // 服务器连接断开
            println!("Disconnected from server");
            self.server_stream = None;
            // 这里可以添加重连逻辑
        } else {
            // P2P连接断开
            // 查找对应的用户ID
            let mut disconnected_user_id = None;
            for (user_id, peer_token) in &self.connected_peers {
                if *peer_token == token {
                    disconnected_user_id = Some(user_id.clone());
                    break;
                }
            }
            
            if let Some(user_id) = disconnected_user_id {
                println!("Disconnected from peer: {}", user_id);
                self.connected_peers.remove(&user_id);
            }
            
            self.streams.remove(&token);
            self.buffers.remove(&token);
        }
        
        Ok(())
    }
    
    // 检查心跳
    fn check_heartbeat(&mut self) -> Result<(), P2PError> {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat).as_secs() >= HEARTBEAT_INTERVAL {
            self.send_heartbeat()?;
            self.last_heartbeat = now;
        }
        
        Ok(())
    }
    
    // 发送心跳
    fn send_heartbeat(&mut self) -> Result<(), P2PError> {
        if self.server_stream.is_some() {
            let heartbeat = Message {
                msg_type: MessageType::Heartbeat,
                sender_id: self.user_id.clone(),
                target_id: None,
                content: None,
                sender_peer_address: "".to_string(),
                sender_listen_port: 0,
                timestamp: SystemTime::now(),
                sender_token: None,
            };
            
            self.send_message(SERVER, &heartbeat)?;
        }
        
        Ok(())
    }
    
    // 请求节点列表
    pub fn request_peer_list(&mut self) -> Result<(), P2PError> {
        println!("[DEBUG] Preparing to request peer list");
        let peer_list_request = Message {
            msg_type: MessageType::PeerListRequest,
            sender_id: self.user_id.clone(),
            target_id: None,
            content: None,
            sender_peer_address: "".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        self.send_message(SERVER, &peer_list_request)?;
        println!("[DEBUG] Sent peer list request");
        Ok(())
    }
    
    // 发送聊天消息
    pub fn send_chat_message(&mut self, target_id: Option<String>, content: String) -> Result<(), P2PError> {
        println!("[DEBUG] Preparing to send message: sender={}, target={:?}, content='{}'", 
                 self.user_id, target_id, content);
        
        // 确保客户端已经初始化
        if self.server_stream.is_none() {
            return Err(P2PError::ConnectionError("Client not connected to server".to_string()));
        }
        
        // 构建聊天消息
        let chat_message = Message {
            msg_type: MessageType::Chat,
            sender_id: self.user_id.clone(),
            target_id: target_id.clone(),
            content: Some(content),
            sender_peer_address: "".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        // 如果指定了目标用户，尝试通过P2P发送
        if let Some(target_id) = &target_id {
            if let Some(peer_token) = self.connected_peers.get(target_id) {
                // 如果已经建立了P2P连接，直接发送
                println!("[DEBUG] Sending message to connected peer: {}", target_id);
                self.send_message(*peer_token, &chat_message)?;
                return Ok(());
            } else if let Some(peer_info) = self.known_peers.get(target_id) {
                println!("[DEBUG] Known peer {} with address {}:{}", target_id, peer_info.address, peer_info.port);
                // 如果知道目标用户的地址但尚未建立连接，先尝试直接连接
                if let Err(e) = self.connect_to_peer(target_id.clone()) {
                    eprintln!("[DEBUG] Failed to connect to peer {} directly: {}, falling back to server relay", target_id, e);
                    // 连接失败，通过服务器转发
                    self.send_message(SERVER, &chat_message)?;
                } else {
                    println!("[DEBUG] Connection to peer {} initiated, sending message through server relay", target_id);
                    // 连接请求已发送，但可能尚未建立，通过服务器转发确保消息送达
                    self.send_message(SERVER, &chat_message)?;
                }
            } else {
                // 未知用户，通过服务器转发
                println!("[DEBUG] Sending message via server relay (unknown peer)");
                self.send_message(SERVER, &chat_message)?;
            }
        } else {
            // 广播消息，通过服务器转发
            println!("[DEBUG] Sending broadcast message via server");
            self.send_message(SERVER, &chat_message)?;
        }
        
        Ok(())
    }
    
    // 发起P2P连接
    pub fn connect_to_peer(&mut self, user_id: String) -> Result<(), P2PError> {
        println!("[DEBUG] Attempting to connect to peer: {}", user_id);
        
        // 检查是否已经连接
        if self.connected_peers.contains_key(&user_id) {
            println!("[DEBUG] Already connected to peer: {}", user_id);
            return Ok(());
        }
        
        // 检查是否知道该用户的地址
        if let Some(peer_info) = self.known_peers.get(&user_id) {
            // 尝试建立P2P连接
            let addr = SocketAddr::new(
                peer_info.address.parse().map_err(|e: std::net::AddrParseError| P2PError::ConnectionError(e.to_string()))?,
                peer_info.port
            );
            
            println!("[DEBUG] Connecting to peer {} at {}:{}", user_id, peer_info.address, peer_info.port);
            
            match TcpStream::connect(addr) {
                Ok(mut stream) => {
                    let token = self.next_token;
                    self.next_token = Token(self.next_token.0 + 1);
                    
                    // 注册新连接的流
                    self.poll.registry()
                        .register(&mut stream, token, Interest::READABLE)
                        .map_err(|e| {
                            eprintln!("[DEBUG] Failed to register peer connection: {}", e);
                            P2PError::IoError(e)
                        })?;
                    
                    self.streams.insert(token, stream);
                    self.buffers.insert(token, Vec::new());
                    self.connected_peers.insert(user_id.clone(), token);
                    
                    println!("[DEBUG] Established P2P connection with {}", user_id);
                    Ok(())
                },
                Err(e) => {
                    eprintln!("[DEBUG] Failed to connect to peer {}: {}", user_id, e);
                    // 连接失败，尝试通过服务器请求连接
                    let connect_request = Message {
                        msg_type: MessageType::ConnectRequest,
                        sender_id: self.user_id.clone(),
                        target_id: Some(user_id.clone()),
                        content: None,
                        sender_peer_address: "".to_string(),
                        sender_listen_port: 0,
                        timestamp: SystemTime::now(),
                        sender_token: None,
                    };
                    
                    if let Err(err) = self.send_message(SERVER, &connect_request) {
                        eprintln!("[DEBUG] Failed to send connect request to server: {}", err);
                        return Err(P2PError::ConnectionError(format!("Failed to connect to peer {} and failed to request connection: {}", user_id, err)));
                    }
                    
                    println!("[DEBUG] Sent connect request to server for peer {}", user_id);
                    // 即使直接连接失败，但我们已经向服务器发送了连接请求，所以返回成功
                    Ok(())
                }
            }
        } else {
            // 不知道用户地址，请求连接信息
            let connect_request = Message {
                msg_type: MessageType::ConnectRequest,
                sender_id: self.user_id.clone(),
                target_id: Some(user_id.clone()),
                content: None,
                sender_peer_address: "".to_string(),
                sender_listen_port: 0,
                timestamp: SystemTime::now(),
                sender_token: None,
            };
            
            self.send_message(SERVER, &connect_request)?;
            println!("[DEBUG] Sent connect request to server for unknown peer {}", user_id);
            Ok(())
        }
    }
    
    // 离开网络
    pub fn leave(&mut self) -> Result<(), P2PError> {
        // 发送离开消息
        let leave_message = Message {
            msg_type: MessageType::Leave,
            sender_id: self.user_id.clone(),
            target_id: None,
            content: None,
            sender_peer_address: "".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            sender_token: None,
        };
        
        self.send_message(SERVER, &leave_message)?;
        
        // 关闭所有连接
        self.server_stream = None;
        self.streams.clear();
        self.buffers.clear();
        self.connected_peers.clear();
        
        println!("Left the network");
        
        Ok(())
    }
}