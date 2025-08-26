use crate::common::*;
use mio::{Events, Interest, Poll, Token};
use mio::net::{TcpListener, TcpStream};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime};
use std::io::{Read, Write};
use crate::common::{Message, MessageType, PeerInfo, P2PError, serialize_message, deserialize_message, MessageSource};

const SERVER: Token = Token(0);
const FIRST_PEER: Token = Token(2);

pub struct P2PServer {
    listener: TcpListener,
    poll: Poll,
    events: Events,
    streams: HashMap<Token, TcpStream>,
    buffers: HashMap<Token, Vec<u8>>,
    peers: HashMap<Token, PeerInfo>,
    user_to_token: HashMap<String, Token>,
    next_token: Token,
    last_heartbeat: Instant,
}

impl P2PServer {
    pub fn new(addr: &str) -> Result<Self, P2PError> {
        let addr: SocketAddr = addr.parse().map_err(|e: std::net::AddrParseError| P2PError::ConnectionError(e.to_string()))?;
        let mut listener = TcpListener::bind(addr)?;
        let poll = Poll::new()?;
        
        poll.registry()
            .register(&mut listener, SERVER, Interest::READABLE)?;
            
        Ok(Self {
            listener,
            poll,
            events: Events::with_capacity(128),
            streams: HashMap::new(),
            buffers: HashMap::new(),
            peers: HashMap::new(),
            user_to_token: HashMap::new(),
            next_token: FIRST_PEER,
            last_heartbeat: Instant::now(),
        })
    }
    
    pub fn start(&mut self) -> Result<(), P2PError> {
        println!("P2P server started on {}", self.listener.local_addr()?);
        
        loop {
            self.poll.poll(&mut self.events, Some(Duration::from_millis(100)))?;
            
            // Collect event information first to avoid borrow conflicts
            let mut server_events = Vec::new();
            let mut readable_tokens = Vec::new();
            let mut writable_tokens = Vec::new();
            
            for event in &self.events {
                match event.token() {
                    SERVER => {
                        if event.is_readable() {
                            server_events.push(event.token());
                        }
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
            
            // Process server events
            for _token in server_events {
                self.accept_new_connection()?;
            }
            
            // Process readable events
            for token in readable_tokens {
                self.handle_readable(token)?;
            }
            
            // Process writable events
            for token in writable_tokens {
                self.handle_writable(token)?;
            }
            
            self.check_heartbeat()?;
            self.check_peer_timeouts()?;
        }
    }
    
    fn accept_new_connection(&mut self) -> Result<(), P2PError> {
        match self.listener.accept() {
            Ok((mut stream, addr)) => {
                let token = self.next_token;
                self.next_token = Token(self.next_token.0 + 1);
                
                self.poll.registry()
                    .register(&mut stream, token, Interest::READABLE)?;
                
                self.streams.insert(token, stream);
                self.buffers.insert(token, Vec::new());
                
                println!("New client connected: {}", addr);
            },
            Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => return Err(P2PError::IoError(e)),
            _ => {}
        }
        Ok(())
    }
    
    fn handle_readable(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            let mut buffer = [0; 1024];
            match stream.read(&mut buffer) {
                Ok(0) => self.remove_peer(token),
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
            self.handle_message(&message, token)?;
        }
        
        Ok(())
    }
    
    fn handle_message(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
        match message.msg_type {
            MessageType::Join => self.handle_join_message(message, token)?,
            MessageType::Leave => self.handle_leave_message(message, token)?,
            MessageType::Chat => self.handle_chat_message(message)?,
            MessageType::Heartbeat => self.handle_heartbeat_message(token)?,
            MessageType::PeerListRequest => self.handle_peer_list_request(token)?,
            MessageType::ConnectRequest => self.handle_connect_request(message, token)?,
            _ => println!("Unknown message type: {:?}", message.msg_type),
        }
        Ok(())
    }
    
    fn handle_join_message(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
        let user_id = &message.sender_id;
        let peer_info = PeerInfo::new(
            user_id.clone(),
            message.sender_peer_address.clone(),
            message.sender_listen_port
        );
        
        self.peers.insert(token, peer_info.clone());
        self.user_to_token.insert(user_id.clone(), token);
        
        println!("User {} joined", user_id);
        
        // Notify other users
        let join_notification = Message {
            msg_type: MessageType::UserJoined,
            sender_id: user_id.clone(),
            target_id: None,
            content: Some(user_id.clone()),
            sender_peer_address: message.sender_peer_address.clone(),
            sender_listen_port: message.sender_listen_port,
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        };
        
        let peer_tokens: Vec<Token> = self.peers.keys().filter(|&t| *t != token).cloned().collect();
        for peer_token in peer_tokens {
            self.send_message(peer_token, &join_notification)?;
        }
        
        self.send_peer_list(token)?;
        Ok(())
    }
    
    fn handle_leave_message(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
        let user_id = &message.sender_id;
        self.remove_peer(token);
        
        println!("User {} left", user_id);
        
        let leave_notification = Message {
            msg_type: MessageType::UserLeft,
            sender_id: user_id.clone(),
            target_id: None,
            content: Some(user_id.clone()),
            sender_peer_address: String::new(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        };
        
        let peer_tokens: Vec<Token> = self.peers.keys().cloned().collect();
        for peer_token in peer_tokens {
            self.send_message(peer_token, &leave_notification)?;
        }
        
        Ok(())
    }
    
    fn handle_chat_message(&mut self, message: &Message) -> Result<(), P2PError> {
        if let Some(target_id) = &message.target_id {
            if let Some(token) = self.user_to_token.get(target_id) {
                self.send_message(*token, message)?;
            }
        } else {
            let peer_tokens: Vec<Token> = self.peers.keys().cloned().collect();
            for token in peer_tokens {
                self.send_message(token, message)?;
            }
        }
        Ok(())
    }
    
    fn handle_heartbeat_message(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(peer_info) = self.peers.get_mut(&token) {
            peer_info.last_heartbeat = Instant::now();
        }
        Ok(())
    }
    
    fn handle_peer_list_request(&mut self, token: Token) -> Result<(), P2PError> {
        self.send_peer_list(token)?;
        Ok(())
    }
    
    fn handle_connect_request(&mut self, message: &Message, token: Token) -> Result<(), P2PError> {
        if let Some(target_id) = &message.target_id {
            if let Some(target_token) = self.user_to_token.get(target_id) {
                if let Some(peer_info) = self.peers.get(target_token) {
                    let content = format!("{},{}", peer_info.address, peer_info.port);
                    let connect_response = Message {
                        msg_type: MessageType::ConnectResponse,
                        sender_id: peer_info.user_id.clone(),
                        target_id: Some(message.sender_id.clone()),
                        content: Some(content),
                        sender_peer_address: peer_info.address.clone(),
                        sender_listen_port: peer_info.port,
                        timestamp: SystemTime::now(),
                        source: MessageSource::Server,
                    };
                    
                    self.send_message(token, &connect_response)?;
                }
            }
        }
        Ok(())
    }
    
    fn handle_writable(&mut self, token: Token) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            if let Some(buffer) = self.buffers.get_mut(&token) {
                if !buffer.is_empty() {
                    match stream.write_all(buffer) {
                        Ok(()) => {
                            buffer.clear();
                            // Switch back to read-only mode
                            self.poll.registry()
                                .reregister(stream, token, Interest::READABLE)?;
                        }
                        Err(e) if e.kind() != std::io::ErrorKind::WouldBlock => {
                            self.remove_peer(token);
                            return Err(e.into());
                        }
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
    
    fn send_message(&mut self, token: Token, message: &Message) -> Result<(), P2PError> {
        if let Some(stream) = self.streams.get_mut(&token) {
            let data = serialize_message(message)?;
            
            // Try to write immediately
            match stream.write_all(&data) {
                Ok(()) => {
                    // Message sent successfully
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Buffer the message for later
                    if let Some(buffer) = self.buffers.get_mut(&token) {
                        buffer.extend_from_slice(&data);
                        self.poll.registry()
                            .reregister(stream, token, Interest::READABLE | Interest::WRITABLE)?;
                    }
                }
                Err(e) => {
                    self.remove_peer(token);
                    return Err(P2PError::IoError(e));
                }
            }
        }
        Ok(())
    }
    
    fn remove_peer(&mut self, token: Token) {
        if let Some(peer_info) = self.peers.remove(&token) {
            self.user_to_token.remove(&peer_info.user_id);
        }
        self.streams.remove(&token);
        self.buffers.remove(&token);
        println!("Removed peer: {:?}", token);
    }
    
    fn send_peer_list(&mut self, token: Token) -> Result<(), P2PError> {
        let peer_list: Vec<_> = self.peers.values()
            .map(|info| (info.user_id.clone(), info.address.clone(), info.port))
            .collect();
        
        let peer_list_data = serde_json::to_vec(&peer_list)?;
        
        let peer_list_message = Message {
            msg_type: MessageType::PeerList,
            sender_id: "SERVER".to_string(),
            target_id: None,
            content: Some(String::from_utf8_lossy(&peer_list_data).to_string()),
            sender_peer_address: String::new(),
            sender_listen_port: 0,
            timestamp: SystemTime::now(),
            source: MessageSource::Server,
        };
        
        self.send_message(token, &peer_list_message)?;
        Ok(())
    }
    
    fn check_heartbeat(&mut self) -> Result<(), P2PError> {
        let now = Instant::now();
        if now.duration_since(self.last_heartbeat) > Duration::from_secs(30) {
            let heartbeat_message = Message {
                msg_type: MessageType::Heartbeat,
                sender_id: "SERVER".to_string(),
                target_id: None,
                content: None,
                sender_peer_address: String::new(),
                sender_listen_port: 0,
                timestamp: SystemTime::now(),
                source: MessageSource::Server,
            };
            
            let peer_tokens: Vec<Token> = self.peers.keys().cloned().collect();
            for token in peer_tokens {
                self.send_message(token, &heartbeat_message)?;
            }
            self.last_heartbeat = now;
        }
        Ok(())
    }
    
    fn check_peer_timeouts(&mut self) -> Result<(), P2PError> {
        let now = Instant::now();
        let timeout_duration = Duration::from_secs(60);
        
        let timeout_tokens: Vec<_> = self.peers.iter()
            .filter(|(_, info)| now.duration_since(info.last_heartbeat) > timeout_duration)
            .map(|(token, _)| *token)
            .collect();
        
        for token in timeout_tokens {
            self.remove_peer(token);
        }
        
        Ok(())
    }
}
