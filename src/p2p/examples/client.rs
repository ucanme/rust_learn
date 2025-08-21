use p2p::client::P2PClient;
use p2p::common::P2PError;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::io::{self, BufRead};

// 定义消息类型，用于线程间通信
#[derive(Debug)]
enum ClientCommand {
    BroadcastMessage(String),
    PrivateMessage(String, String),
    Exit
}

fn main() -> Result<(), P2PError> {
    println!("Starting P2P client...");
    
    // 获取用户输入的用户ID
    println!("Please enter your user ID: ");
    let mut user_id = String::new();
    io::stdin().lock().read_line(&mut user_id)?;
    let user_id = user_id.trim().to_string();
    
    // 创建P2P客户端，连接到本地8080端口的服务端
    // 使用动态端口分配，让系统自动选择可用端口
    let local_listen_port = 0; // 使用0让系统分配可用端口
    let client = P2PClient::new("127.0.0.1:8888", local_listen_port, user_id.clone())?;
    
    // 将客户端包装在Arc<Mutex>中以便线程间安全共享
    let client_arc = Arc::new(Mutex::new(client));
    
    // 创建通道用于线程间通信
    let (command_sender, command_receiver) = mpsc::channel::<ClientCommand>();
    
    println!("P2P client created successfully, connecting to server...");
    
    // 在单独的线程中启动客户端的事件循环
    let client_arc_clone = Arc::clone(&client_arc);
    let _event_loop_thread = thread::spawn(move || {
        println!("[DEBUG] Event loop thread started");
        
        loop {
            // 尝试获取锁，如果失败则短暂等待后重试
            match client_arc_clone.try_lock() {
                Ok(mut client) => {
                    // 检查是否已经初始化
                    if !client.is_initialized() {
                        // 首次获取锁时执行初始化操作
                        println!("[DEBUG] Initializing client");
                        if let Err(e) = client.initialize() {
                            eprintln!("Client initialization error: {}", e);
                            // 如果初始化失败，等待一段时间后重试
                            std::thread::sleep(std::time::Duration::from_millis(100));
                        }
                    } else {
                        // 已经初始化，执行单次事件循环
                        if let Err(e) = client.poll_once() {
                            eprintln!("Client event loop error: {}", e);
                            // 如果发生连接错误，可能需要重新初始化
                            if let P2PError::ConnectionError(_) = e {
                                println!("[DEBUG] Connection error detected, resetting client state");
                                // 不直接修改initialized标志，而是在下一次循环中通过检查server_stream来判断
                            }
                        }
                    }
                },
                Err(_) => {
                    // 无法获取锁，短暂睡眠后重试
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
            
            // 短暂睡眠以避免CPU占用过高
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    });
    
    // 在另一个线程中处理来自主线程的命令
    let client_arc_clone = Arc::clone(&client_arc);
    let command_handler_thread = thread::spawn(move || {
        handle_commands(client_arc_clone, command_receiver);
    });
    
    // 主线程处理用户输入
    println!("Connected to server! Type messages to broadcast, or type '/exit' to quit.");
    println!("To send a private message, use: /msg <user_id> <message>");
    
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        if let Ok(input) = line {
            if input.trim() == "/exit" {
                println!("Exiting...");
                // 发送退出命令到处理线程
                println!("[DEBUG] Sending Exit command to handler thread");
                if let Err(e) = command_sender.send(ClientCommand::Exit) {
                    eprintln!("[ERROR] Failed to send Exit command: {}", e);
                }
                break;
            } else if input.starts_with("/msg ") {
                // 发送私聊消息
                let parts: Vec<&str> = input.trim().splitn(3, ' ').collect();
                if parts.len() >= 3 {
                    let target_id = parts[1].to_string();
                    let content = parts[2].to_string();
                    println!("Sending private message to {}: {}", target_id, content);
                    println!("[DEBUG] Sending PrivateMessage command to handler thread");
                    if let Err(e) = command_sender.send(ClientCommand::PrivateMessage(target_id.clone(), content.clone())) {
                        eprintln!("[ERROR] Failed to send PrivateMessage command: {}", e);
                    } else {
                        println!("[DEBUG] PrivateMessage command sent successfully");
                    }
                } else {
                    println!("Invalid private message format. Use: /msg <user_id> <message>");
                }
            } else {
                // 发送广播消息
                println!("Sending broadcast message: {}", input);
                println!("[DEBUG] Sending BroadcastMessage command to handler thread");
                if let Err(e) = command_sender.send(ClientCommand::BroadcastMessage(input.clone())) {
                    eprintln!("[ERROR] Failed to send BroadcastMessage command: {}", e);
                } else {
                    println!("[DEBUG] BroadcastMessage command sent successfully");
                }
            }
        }
    }
    
    // 等待线程结束
    if let Err(e) = command_handler_thread.join() {
        eprintln!("Failed to join command handler thread: {:?}", e);
    }
    
    // 注意：事件循环线程可能不会正常结束，因为它在无限循环中运行
    // 这里不等待事件循环线程，直接退出程序
    
    Ok(())
}

// 处理来自主线程的命令
fn handle_commands(client_arc: Arc<Mutex<P2PClient>>, receiver: Receiver<ClientCommand>) {
    println!("[DEBUG] Command handler thread started, waiting for commands...");
    while let Ok(command) = receiver.recv() {
        println!("[DEBUG] Command Received command: {:?}", command);
        match command {
            ClientCommand::BroadcastMessage(content) => {
                println!("[DEBUG] Handling BroadcastMessage command");
                match client_arc.lock() {
                    Ok(mut client) => {
                        println!("[DEBUG] Acquired client lock, preparing to send broadcast message");
                        println!("[DEBUG] Sending broadcast message: '{}'", content);
                        match client.send_chat_message(None, content.clone()) {
                            Ok(_) => {
                                println!("[DEBUG] Broadcast message sent successfully");
                                // 请求节点列表以确保客户端知道所有其他节点
                                if let Err(e) = client.request_peer_list() {
                                    eprintln!("[ERROR] Failed to request peer list: {}", e);
                                } else {
                                    println!("[DEBUG] Peer list requested successfully");
                                }
                            },
                            Err(e) => eprintln!("[ERROR] Failed to send broadcast message: {}", e)
                        }
                    },
                    Err(e) => eprintln!("[ERROR] Failed to acquire client lock: {}", e)
                }
            },
            ClientCommand::PrivateMessage(target_id, content) => {
                println!("[DEBUG] Handling PrivateMessage command to user: {}", target_id);
                match client_arc.lock() {
                    Ok(mut client) => {
                        println!("[DEBUG] Acquired client lock, preparing to send private message");
                        println!("[DEBUG] Sending private message to {}: '{}'", target_id, content);
                        match client.send_chat_message(Some(target_id.clone()), content.clone()) {
                            Ok(_) => {
                                println!("[DEBUG] Private message to {} sent successfully", target_id);
                                // 主动尝试连接目标用户
                                println!("[DEBUG] Attempting to connect to peer: {}", target_id);
                                if let Err(e) = client.connect_to_peer(target_id.clone()) {
                                    eprintln!("[ERROR] Failed to connect to peer {}: {}", target_id, e);
                                } else {
                                    println!("[DEBUG] Connection to peer {} initiated successfully", target_id);
                                }
                            },
                            Err(e) => eprintln!("[ERROR] Failed to send private message: {}", e)
                        }
                    },
                    Err(e) => eprintln!("[ERROR] Failed to acquire client lock: {}", e)
                }
            },
            ClientCommand::Exit => {
                // 退出命令处理
                println!("[DEBUG] Handling Exit command");
                break;
            },
            _ =>{
                println!("[WARN] Unknown command received");
            }
        }
    }
    println!("[DEBUG] Command handler thread exiting");
}