use crate::common::*;
use mio::{Events, Interest, Poll, Token};
use mio::net::{TcpStream, TcpListener};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime, Instant};
use std::io::{Read, Write};
use std::sync::mpsc;
use crate::common::{Message, MessageType, PeerInfo, P2PError, serialize_message, deserialize_message, MessageSource};

const SERVER: Token = Token(0);
const LISTENER: Token = Token(1); // 客户端监听器token

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
    ConnectToPeer(String),  // 连接到指定的peer
    SendDirectMessage(String, String),  // (peer_id, content)
    SmartSendMessage(Option<String>, String),  // 智能发送消息（自动P2P或服务器）
    ListPeers,  // 显示已知对等节点列表
    ShowStatus,  // 显示连接状态
    RefreshPeers,  // 刷新对等节点列表
}

pub struct P2PClient {
    poll: Poll,
    events: Events,
    server_stream: Option<TcpStream>,
    listener: Option<TcpListener>,  // 客户端监听器
    listen_port: u16,  // 实际监听端口
    streams: HashMap<Token, TcpStream>,
    buffers: HashMap<Token, Vec<u8>>,
    user_id: String,
    server_addr: SocketAddr,
    known_peers: HashMap<String, PeerInfo>,
    // P2P连接管理
    peer_to_token: HashMap<String, Token>,  // peer_id -> token 映射
    next_peer_token: Token,  // 下一个可用的peer token
    // 消息发送通道
    message_sender: mpsc::Sender<PendingMessage>,
    message_receiver: mpsc::Receiver<PendingMessage>,
    // 控制指令通道
    control_sender: mpsc::Sender<ClientCommand>,
    control_receiver: mpsc::Receiver<ClientCommand>,
    // 心跳管理
    last_heartbeat: Instant,
}

impl P2PClient {
    pub fn new(server_addr: &str, local_port: u16, user_id: String) -> Result<Self, P2PError> {
        let server_addr: SocketAddr = server_addr.parse().map_err(|e: std::net::AddrParseError| P2PError::ConnectionError(e.to_string()))?;
        let poll = Poll::new()?;
        
        // 创建客户端监听器
        let listen_addr = if local_port == 0 {
            "127.0.0.1:0".parse().unwrap() // 系统分配端口
        } else {
            format!("127.0.0.1:{}", local_port).parse().map_err(|e: std::net::AddrParseError| P2PError::ConnectionError(e.to_string()))?
        };
        
        let mut listener = TcpListener::bind(listen_addr)?;
        let actual_addr = listener.local_addr()?;
        let listen_port = actual_addr.port();
        
        // 注册监听器
        poll.registry().register(&mut listener, LISTENER, Interest::READABLE)?;
        
        // 创建消息发送通道
        let (message_sender, message_receiver) = mpsc::channel();
        // 创建控制指令通道
        let (control_sender, control_receiver) = mpsc::channel();
        
        println!("🚀 客户端监听端口: {}", listen_port);
        
        Ok(Self {
            poll,
            events: Events::with_capacity(1024),
            server_stream: None,
            listener: Some(listener),
            listen_port,
            streams: HashMap::new(),
            buffers: HashMap::new(),
            user_id,
            server_addr,
            known_peers: HashMap::new(),
            peer_to_token: HashMap::new(),
            next_peer_token: Token(1000), // 从1000开始为peer分配（避开LISTENER的token）
            message_sender,
            message_receiver,
            control_sender,
            control_receiver,
            last_heartbeat: Instant::now(),
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
    
    /// 创建智能路由的聊天消息（供外部使用）
    pub fn create_smart_chat_message(&self, target_id: Option<String>, content: String) -> PendingMessage {
        // 如果有目标用户且已建立P2P连接，则通过P2P发送
        if let Some(ref target) = target_id {
            if let Some(&peer_token) = self.peer_to_token.get(target) {
                let message = Message {
                    msg_type: MessageType::Chat,
                    sender_id: self.user_id.clone(),
                    target_id: target_id.clone(),
                    content: Some(content),
                    sender_peer_address: "127.0.0.1".to_string(),
                    sender_listen_port: self.listen_port,
                    timestamp: SystemTime::now(),
                    source: MessageSource::Peer,
                };
                
                return PendingMessage {
                    target: MessageTarget::Peer(peer_token),
                    message,
                };
            }
        }
        
        // 否则通过服务器发送
        let message = Message {
            msg_type: MessageType::Chat,
            sender_id: self.user_id.clone(),
            target_id,
            content: Some(content),
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        };
        
        PendingMessage {
            target: MessageTarget::Server,
            message,
        }
    }
    
    /// 静态方法：创建聊天消息（不需要客户端实例） - 始终通过服务器
    pub fn create_chat_message_static(user_id: String, target_id: Option<String>, content: String) -> PendingMessage {
        let message = Message {
            msg_type: MessageType::Chat,
            sender_id: user_id,
            target_id,
            content: Some(content),
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        };
        
        PendingMessage {
            target: MessageTarget::Server,
            message,
        }
    }
    
    /// 智能发送消息（自动选择P2P或服务器）
    pub fn send_smart_message(&self, target_id: Option<String>, content: String) -> Result<(), P2PError> {
        let pending_message = self.create_smart_chat_message(target_id.clone(), content.clone());
        
        // 根据消息目标显示不同的提示
        match &pending_message.target {
            MessageTarget::Peer(_) => {
                if let Some(target) = &target_id {
                    println!("🚀 [P2P直发 -> {}]: {}", target, content);
                }
            }
            MessageTarget::Server => {
                if let Some(target) = &target_id {
                    println!("📡 [你 -> {}]: {}", target, content);
                } else {
                    println!("📢 [你]: {}", content);
                }
            }
        }
        
        self.message_sender.send(pending_message)
            .map_err(|_| P2PError::ConnectionError("消息发送通道已关闭".to_string()))?;
        Ok(())
    }

    pub fn connect(&mut self) -> Result<(), P2PError> {
        let mut stream = TcpStream::connect(self.server_addr)?;
        self.poll.registry()
            .register(&mut stream, SERVER, Interest::READABLE | Interest::WRITABLE)?;
        
        self.server_stream = Some(stream);
        self.buffers.insert(SERVER, Vec::new());

        // 使用通道发送join消息，包含真实的监听端口
        let join_message = Message {
            msg_type: MessageType::Join,
            sender_id: self.user_id.clone(),
            target_id: None,
            content: None,
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: self.listen_port,  // 发送真实的监听端口
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        };

        self.queue_message(MessageTarget::Server, join_message)?;
        Ok(())
    }

    /// 请求对等节点列表
    pub fn request_peer_list(&self) -> Result<(), P2PError> {
        let request_message = Message {
            msg_type: MessageType::PeerListRequest,
            sender_id: self.user_id.clone(),
            target_id: None,
            content: None,
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        };
        
        self.queue_message(MessageTarget::Server, request_message)?;
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
    
    /// 检查是否连接到服务器
    pub fn is_connected(&self) -> bool {
        self.server_stream.is_some()
    }
    
    /// 尝试重新连接到服务器
    pub fn try_reconnect(&mut self) -> Result<(), P2PError> {
        if self.is_connected() {
            return Ok(()); // 已经连接
        }
        
        println!("尝试重新连接到服务器...");
        
        match TcpStream::connect(self.server_addr) {
            Ok(mut stream) => {
                self.poll.registry()
                    .register(&mut stream, SERVER, Interest::READABLE | Interest::WRITABLE)?;
                
                self.server_stream = Some(stream);
                self.buffers.insert(SERVER, Vec::new());
                
                // 重新发送join消息，包含真实的监听端口
                let join_message = Message {
                    msg_type: MessageType::Join,
                    sender_id: self.user_id.clone(),
                    target_id: None,
                    content: None,
                    sender_peer_address: "127.0.0.1".to_string(),
                    sender_listen_port: self.listen_port,  // 发送真实的监听端口
                    timestamp: SystemTime::now(),
                    source: MessageSource::Server,
                };
                
                self.queue_message(MessageTarget::Server, join_message)?;
                println!("重新连接成功！");
                Ok(())
            }
            Err(e) => {
                eprintln!("重新连接失败: {}", e);
                Err(P2PError::IoError(e))
            }
        }
    }
    
    /// 运行客户端（纯粹的网络事件循环）
    /// 使用通道接收外部指令和消息
    pub fn run(&mut self) -> Result<(), P2PError> {
        println!("客户端开始运行，按 Ctrl+C 或输入 /exit 退出");
        let mut reconnect_attempts = 0;
        let max_reconnect_attempts = 5;
        
        loop {
            // 检查连接状态，如果断开则尝试重连
            if !self.is_connected() && reconnect_attempts < max_reconnect_attempts {
                if let Err(_) = self.try_reconnect() {
                    reconnect_attempts += 1;
                    println!("重连尝试 {}/{}", reconnect_attempts, max_reconnect_attempts);
                    std::thread::sleep(Duration::from_secs(2)); // 等待一段时间再重试
                    continue;
                } else {
                    reconnect_attempts = 0; // 重连成功，重置计数器
                }
            }
            
            // 处理网络事件和待发送消息
            match self.poll.poll(&mut self.events, Some(Duration::from_millis(50))) {
                Ok(_) => {
                    if let Err(e) = self.process_events() {
                        eprintln!("处理事件时出错: {}", e);
                        // 不要因为处理事件错误就退出，继续尝试
                        continue;
                    }
                }
                Err(e) => {
                    eprintln!("轮询事件时出错: {}", e);
                    // 短暂休眠后继续尝试
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            }
            
            // 检查是否需要发送心跳
            self.check_and_send_heartbeat();
            
            // 检查控制指令
            match self.control_receiver.try_recv() {
                Ok(ClientCommand::Stop) => {
                    println!("收到停止指令，正在关闭客户端...");
                    break;
                }
                Ok(ClientCommand::ConnectToPeer(peer_id)) => {
                    if let Err(e) = self.connect_to_peer(&peer_id) {
                        eprintln!("连接到对等节点 {} 失败: {}", peer_id, e);
                    }
                }
                Ok(ClientCommand::SendDirectMessage(peer_id, content)) => {
                    if let Err(e) = self.send_direct_message(&peer_id, content) {
                        eprintln!("发送直接消息失败: {}", e);
                    }
                }
                Ok(ClientCommand::SmartSendMessage(target_id, content)) => {
                    if let Err(e) = self.send_smart_message(target_id, content) {
                        eprintln!("发送消息失败: {}", e);
                    }
                }
                Ok(ClientCommand::ListPeers) => {
                    self.list_known_peers();
                }
                Ok(ClientCommand::ShowStatus) => {
                    self.show_status();
                }
                Ok(ClientCommand::RefreshPeers) => {
                    if let Err(e) = self.request_peer_list() {
                        eprintln!("刷新对等节点列表失败: {}", e);
                    } else {
                        println!("🔄 已请求刷新对等节点列表...");
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // 没有指令，继续运行
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    println!("控制通道已断开，客户端退出");
                    break;
                }
            }
            
            // 如果重连尝试过多，给出提示
            if reconnect_attempts >= max_reconnect_attempts {
                eprintln!("达到最大重连尝试次数，客户端将在断线模式下继续运行");
                reconnect_attempts = 0; // 重置以便稍后再次尝试
                std::thread::sleep(Duration::from_secs(5));
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
                LISTENER => self.handle_listener_event()?,
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
                Ok(0) => {
                    println!("⚠️ 服务器主动断开连接，将尝试重新连接...");
                    self.server_stream = None;
                    self.buffers.remove(&SERVER);
                    return Ok(());
                }
                Ok(n) => {
                    if let Some(peer_buffer) = self.buffers.get_mut(&SERVER) {
                        peer_buffer.extend_from_slice(&buffer[..n]);
                    }
                    self.try_parse_messages(SERVER)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // 这是正常的非阻塞状态，不用处理
                }
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset || 
                         e.kind() == std::io::ErrorKind::ConnectionAborted ||
                         e.kind() == std::io::ErrorKind::BrokenPipe => {
                    println!("⚠️ 服务器连接被重置/中止: {}，将尝试重新连接...", e);
                    self.server_stream = None;
                    self.buffers.remove(&SERVER);
                    return Ok(());
                }
                Err(e) => {
                    // 其他类型的错误，记录但不立即断开连接
                    eprintln!("⚠️ 服务器连接出现错误: {}，继续监听...", e);
                    // 只有在持续错误时才断开连接
                }
            }
        }
        Ok(())
    }

    /// 处理监听器事件，接受其他客户端的P2P连接
    fn handle_listener_event(&mut self) -> Result<(), P2PError> {
        if let Some(listener) = &self.listener {
            loop {
                match listener.accept() {
                    Ok((mut stream, addr)) => {
                        let peer_token = self.next_peer_token;
                        self.next_peer_token = Token(self.next_peer_token.0 + 1);
                        
                        self.poll.registry()
                            .register(&mut stream, peer_token, Interest::READABLE | Interest::WRITABLE)?;
                        
                        self.streams.insert(peer_token, stream);
                        self.buffers.insert(peer_token, Vec::new());
                        
                        println!("🎉 接受到P2P连接: {} (Token: {:?})", addr, peer_token);
                    }
                    Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => {
                        eprintln!("接受P2P连接错误: {}", e);
                        return Err(P2PError::IoError(e));
                    }
                    _ => break,
                }
            }
        }
        Ok(())
    }

    fn handle_readable(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            let mut buffer = [0; 1024];
            match stream.read(&mut buffer) {
                Ok(0) => {
                    println!("对等节点 {:?} 已断开连接", token);
                    self.remove_peer(token);
                }
                Ok(n) => {
                    if let Some(peer_buffer) = self.buffers.get_mut(&token) {
                        peer_buffer.extend_from_slice(&buffer[..n]);
                    }
                    self.try_parse_messages(token)?;
                }
                Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => {
                    eprintln!("对等节点 {:?} 连接错误: {}", token, e);
                    self.remove_peer(token);
                    return Ok(()); // 不要因为一个对等节点的错误就退出
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
                
                if let Ok(mut message) = deserialize_message(message_data) {
                    // 根据token来源设置消息来源标识
                    message.source = if token == SERVER {
                        MessageSource::Server
                    } else {
                        MessageSource::Peer
                    };
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
                    // 根据消息来源显示不同的标识
                    let source_tag = match message.source {
                        MessageSource::Server => "[服务器]",
                        MessageSource::Peer => "[P2P]",
                    };
                    
                    // 检查是否为私聊消息
                    if message.target_id.is_some() {
                        println!("{}私聊[{}]: {}", source_tag, message.sender_id, content);
                    } else {
                        println!("{}公共[{}]: {}", source_tag, message.sender_id, content);
                    }
                }
            }
            MessageType::PeerList => {
                if let Some(content) = &message.content {
                    println!("📄 收到对等节点列表: {}", content);
                    if let Ok(peer_list) = serde_json::from_str::<Vec<(String, String, u16)>>(content) {
                        println!("🗺️ 解析到 {} 个对等节点:", peer_list.len());
                        for (user_id, address, port) in peer_list {
                            if user_id != self.user_id {
                                let peer_info = PeerInfo::new(user_id.clone(), address.clone(), port);
                                self.known_peers.insert(peer_info.user_id.clone(), peer_info);
                                println!("  ✅ 添加对等节点: {} ({}:{})", user_id, address, port);
                            } else {
                                println!("  ℹ️ 跳过自己: {} ({}:{})", user_id, address, port);
                            }
                        }
                        println!("📊 当前已知对等节点数量: {}", self.known_peers.len());
                    } else {
                        eprintln!("❌ 无法解析对等节点列表");
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
            match stream.write_all(&data) {
                Ok(_) => {
                    // 消息发送成功
                    Ok(())
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // 非阻塞错误，稍后重试
                    eprintln!("⚠️ 连接忙碌，稍后重试...");
                    std::thread::sleep(Duration::from_millis(50));
                    stream.write_all(&data).map_err(P2PError::IoError)
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotConnected => {
                    eprintln!("❌ 连接未建立或已断开: {}", e);
                    Err(P2PError::IoError(e))
                }
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe || 
                         e.kind() == std::io::ErrorKind::ConnectionReset => {
                    eprintln!("❌ P2P连接已断开: {}", e);
                    // 清理断开的连接
                    self.remove_peer(token);
                    Err(P2PError::IoError(e))
                }
                Err(e) => {
                    eprintln!("❌ 发送P2P消息错误: {}", e);
                    Err(P2PError::IoError(e))
                }
            }
        } else {
            eprintln!("❌ 找不到对等节点连接 (Token: {:?})", token);
            Err(P2PError::PeerNotFound)
        }
    }

    fn remove_peer(&mut self, token: Token) {
        // 从映射中移除
        let peer_id = self.peer_to_token.iter()
            .find(|(_, &t)| t == token)
            .map(|(id, _)| id.clone());
        
        if let Some(peer_id) = peer_id {
            self.peer_to_token.remove(&peer_id);
            println!("🚫 P2P连接已断开: {}", peer_id);
        }
        
        self.streams.remove(&token);
        self.buffers.remove(&token);
    }

    /// 直接连接到指定的对等节点
    pub fn connect_to_peer(&mut self, peer_id: &str) -> Result<(), P2PError> {
        println!("🔍 尝试连接到对等节点: {}", peer_id);
        println!("📋 当前已知对等节点数量: {}", self.known_peers.len());
        
        for (id, info) in &self.known_peers {
            println!("  📍 {}: {}:{}", id, info.address, info.port);
        }
        
        // 检查是否尝试连接到自己
        if peer_id == self.user_id {
            eprintln!("❌ 不能连接到自己！");
            return Err(P2PError::ConnectionError("不能连接到自己".to_string()));
        }
        
        // 检查是否已经连接
        if self.peer_to_token.contains_key(peer_id) {
            println!("ℹ️ 已经与对等节点 {} 建立了直接连接", peer_id);
            return Ok(());
        }
        
        if let Some(peer_info) = self.known_peers.get(peer_id) {
            let peer_addr = peer_info.socket_addr()?;
            println!("🌐 尝试连接到 {}", peer_addr);
            
            match TcpStream::connect(peer_addr) {
                Ok(mut stream) => {
                    let peer_token = self.next_peer_token;
                    self.next_peer_token = Token(self.next_peer_token.0 + 1);
                    
                    // 先注册到事件循环
                    self.poll.registry()
                        .register(&mut stream, peer_token, Interest::READABLE | Interest::WRITABLE)?;
                    
                    self.streams.insert(peer_token, stream);
                    self.buffers.insert(peer_token, Vec::new());
                    self.peer_to_token.insert(peer_id.to_string(), peer_token);
                    
                    println!("✨ 已直接连接到对等节点: {} (Token: {:?})", peer_id, peer_token);
                    
                    // 等待一小段时间确保连接稳定
                    std::thread::sleep(Duration::from_millis(100));
                    
                    Ok(())
                }
                Err(e) => {
                    eprintln!("❌ 无法连接到对等节点 {}: {}", peer_id, e);
                    Err(P2PError::IoError(e))
                }
            }
        } else {
            eprintln!("❌ 未知的对等节点: {} (请检查对等节点是否在线)", peer_id);
            Err(P2PError::PeerNotFound)
        }
    }
    
    /// 发送直接P2P消息
    pub fn send_direct_message(&mut self, peer_id: &str, content: String) -> Result<(), P2PError> {
        // 检查是否尝试连接到自己
        if peer_id == self.user_id {
            eprintln!("❌ 不能发送消息给自己！");
            return Err(P2PError::ConnectionError("不能发送消息给自己".to_string()));
        }
        
        // 查找是否已经有直接连接
        let peer_token = self.find_peer_token(peer_id);
        
        if peer_token.is_none() {
            // 如果没有直接连接，尝试建立连接
            println!("🔗 正在为 {} 建立 P2P 连接...", peer_id);
            self.connect_to_peer(peer_id)?;
            
            // 重新查找连接
            let peer_token = self.find_peer_token(peer_id).ok_or(P2PError::PeerNotFound)?;
            
            // 等待连接稳定后发送消息
            println!("⏳ 等待连接稳定...");
            std::thread::sleep(Duration::from_millis(200));
            
            return self.send_p2p_message_with_retry(peer_token, peer_id, content);
        }
        
        let peer_token = peer_token.unwrap();
        self.send_p2p_message_with_retry(peer_token, peer_id, content)
    }
    
    /// 查找对等节点的token
    fn find_peer_token(&self, peer_id: &str) -> Option<Token> {
        self.peer_to_token.get(peer_id).copied()
    }
    
    /// 显示已知对等节点列表
    fn list_known_peers(&self) {
        println!("🗺️ 已知对等节点列表 ({} 个):", self.known_peers.len());
        if self.known_peers.is_empty() {
            println!("  ℹ️ 暂无已知对等节点");
        } else {
            for (id, info) in &self.known_peers {
                let connection_status = if self.peer_to_token.contains_key(id) {
                    "✅ 已连接"
                } else {
                    "❌ 未连接"
                };
                println!("  {} {}: {}:{}", connection_status, id, info.address, info.port);
            }
        }
        println!("🔗 当前活跃P2P连接数: {}", self.peer_to_token.len());
    }
    
    /// 检查并发送心跳消息
    fn check_and_send_heartbeat(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat) > Duration::from_secs(30) {
            if self.is_connected() {
                let heartbeat_message = Message {
                    msg_type: MessageType::Heartbeat,
                    sender_id: self.user_id.clone(),
                    target_id: None,
                    content: None,
                    sender_peer_address: "127.0.0.1".to_string(),
                    sender_listen_port: self.listen_port,
                    timestamp: SystemTime::now(),
                    source: MessageSource::Server,
                };
                
                if let Ok(_) = self.queue_message(MessageTarget::Server, heartbeat_message) {
                    self.last_heartbeat = now;
                    println!("💓 发送心跳到服务器");
                }
            }
        }
    }
    
    /// 显示连接状态
    fn show_status(&self) {
        println!("📋 ==========  连接状态  ===========");
        println!("👤 用户ID: {}", self.user_id);
        println!("🏠 本地监听端口: {}", self.listen_port);
        println!("🌐 服务器地址: {}", self.server_addr);
        
        let server_status = if self.is_connected() {
            "✅ 已连接"
        } else {
            "❌ 已断开"
        };
        println!("🖥️ 服务器连接: {}", server_status);
        
        let time_since_heartbeat = Instant::now().duration_since(self.last_heartbeat).as_secs();
        println!("💓 上次心跳: {} 秒前", time_since_heartbeat);
        
        println!("🗺️ 已知对等节点: {} 个", self.known_peers.len());
        println!("🔗 活跃P2P连接: {} 个", self.peer_to_token.len());
        println!("========================================");
    }
    
    /// 发送P2P消息的内部方法（带重试机制）
    fn send_p2p_message_with_retry(&mut self, peer_token: Token, peer_id: &str, content: String) -> Result<(), P2PError> {
        let message = Message {
            msg_type: MessageType::Chat,
            sender_id: self.user_id.clone(),
            target_id: Some(peer_id.to_string()),
            content: Some(content.clone()),
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Peer,
        };
        
        // 尝试发送，如果失败则重试
        for attempt in 1..=3 {
            match self.send_message_to_peer(peer_token, &message) {
                Ok(_) => {
                    println!("🚀 [P2P直发 -> {}]: {}", peer_id, content);
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("⚠️ 发送P2P消息尝试 {} 失败: {}", attempt, e);
                    if attempt < 3 {
                        println!("🔄 等待 {}ms 后重试...", attempt * 100);
                        std::thread::sleep(Duration::from_millis((attempt * 100) as u64));
                    } else {
                        eprintln!("❌ P2P消息发送最终失败");
                        return Err(e);
                    }
                }
            }
        }
        
        Err(P2PError::ConnectionError("消息发送超过最大重试次数".to_string()))
    }
    
    /// 发送P2P消息的内部方法（旧版本，保留兼容）
    fn send_p2p_message(&mut self, peer_token: Token, peer_id: &str, content: String) -> Result<(), P2PError> {
        let message = Message {
            msg_type: MessageType::Chat,
            sender_id: self.user_id.clone(),
            target_id: Some(peer_id.to_string()),
            content: Some(content.clone()),
            sender_peer_address: "127.0.0.1".to_string(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Peer,
        };
        
        self.send_message_to_peer(peer_token, &message)?;
        println!("🚀 [P2P直发 -> {}]: {}", peer_id, content);
        Ok(())
    }
}