use p2p::client::{P2PClient, PendingMessage, ClientCommand};
use p2p::common::P2PError;
use std::io::{self, BufRead};
use std::env;
use std::thread;
use std::sync::mpsc;

fn main() -> Result<(), P2PError> {
    let server_addr = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8080".to_string());
    println!("正在连接到P2P服务器: {}...", server_addr);
    
    // 获取用户ID
    print!("请输入您的用户ID: ");
    io::Write::flush(&mut io::stdout()).ok();
    let mut user_id = String::new();
    io::stdin().read_line(&mut user_id)?;
    let user_id = user_id.trim().to_string();
    
    if user_id.is_empty() {
        println!("用户ID不能为空！");
        return Ok(());
    }
    
    // 创建、连接P2P客户端（使用随机端口）
    let mut client = P2PClient::new(&server_addr, 0, user_id.clone())?;
    client.connect()?;
    client.request_peer_list()?;
    
    println!("已连接到服务器！用户: {}", user_id);
    println!("\n使用说明:");
    println!("  直接输入消息发送公共消息");
    println!("  @<用户名> <消息> 发送私聊消息");
    println!("  /list 显示已知对等节点列表");
    println!("  /refresh 刷新对等节点列表");
    println!("  /status 显示连接状态");
    println!("  /p2p <用户名> 建立直接P2P连接");
    println!("  /direct <用户名> <消息> 发送直接P2P消息");
    println!("  /exit 退出客户端\n");
    
    // 获取通道发送器
    let message_sender = client.get_message_sender();
    let control_sender = client.get_control_sender();
    
    // 在单独线程中处理用户输入
    let client_for_input = message_sender.clone();
    let control_for_input = control_sender.clone();
    let user_id_for_input = user_id.clone();
    
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        
        println!("输入线程已启动，可以开始聊天\n");
        
        loop {
            let mut input = String::new();
            match handle.read_line(&mut input) {
                Ok(0) => {
                    // EOF - 通常是 Ctrl+D
                    println!("\n检测到输入结束，正在退出...");
                    let _ = control_for_input.send(ClientCommand::Stop);
                    break;
                }
                Ok(_) => {
                    let input = input.trim();
                    
                    if input.is_empty() {
                        continue;
                    }
                    
                    // 检查退出命令
                    if input.eq_ignore_ascii_case("/exit") {
                        println!("正在退出...");
                        let _ = control_for_input.send(ClientCommand::Stop);
                        break;
                    }
                    
                    // 检查列表命令
                    if input.eq_ignore_ascii_case("/list") {
                        let _ = control_for_input.send(ClientCommand::ListPeers);
                        continue;
                    }
                    
                    // 检查状态命令
                    if input.eq_ignore_ascii_case("/status") {
                        let _ = control_for_input.send(ClientCommand::ShowStatus);
                        continue;
                    }
                    
                    // 检查刷新命令
                    if input.eq_ignore_ascii_case("/refresh") {
                        let _ = control_for_input.send(ClientCommand::RefreshPeers);
                        continue;
                    }
                    
                    // 检查P2P连接命令
                    if let Some(peer_id) = input.strip_prefix("/p2p ") {
                        let peer_id = peer_id.trim();
                        if !peer_id.is_empty() {
                            println!("🔗 正在建立P2P连接到: {}", peer_id);
                            let _ = control_for_input.send(ClientCommand::ConnectToPeer(peer_id.to_string()));
                        } else {
                            println!("格式: /p2p <用户名>");
                        }
                        continue;
                    }
                    
                    // 检查直接消息命令
                    if let Some(direct_msg) = input.strip_prefix("/direct ") {
                        if let Some((peer_id, content)) = direct_msg.split_once(' ') {
                            let peer_id = peer_id.trim();
                            let content = content.trim();
                            if !peer_id.is_empty() && !content.is_empty() {
                                let _ = control_for_input.send(ClientCommand::SendDirectMessage(peer_id.to_string(), content.to_string()));
                            } else {
                                println!("格式: /direct <用户名> <消息>");
                            }
                        } else {
                            println!("格式: /direct <用户名> <消息>");
                        }
                        continue;
                    }
                    
                    // 处理消息发送
                    handle_user_input(&client_for_input, input, &user_id_for_input);
                }
                Err(e) => {
                    eprintln!("读取输入错误: {}", e);
                    println!("输入出错，正在退出...");
                    let _ = control_for_input.send(ClientCommand::Stop);
                    break;
                }
            }
        }
        println!("输入线程已结束");
    });
    
    // 运行客户端 - 现在非常简洁！
    match client.run() {
        Ok(_) => println!("客户端正常退出。"),
        Err(e) => {
            eprintln!("客户端运行出错: {}", e);
            println!("客户端已断开连接。");
        }
    }
    Ok(())
}

/// 处理用户输入的函数（完全基于通道）
fn handle_user_input(
    message_sender: &mpsc::Sender<PendingMessage>, 
    input: &str,
    user_id: &str
) {
    // 处理消息发送
    if let Some(message) = input.strip_prefix('@') {
        if let Some((target, msg)) = message.split_once(' ') {
            let target = target.trim();
            let msg = msg.trim();
            if !target.is_empty() && !msg.is_empty() {
                let pending_message = P2PClient::create_chat_message_static(
                    user_id.to_string(), 
                    Some(target.to_string()), 
                    msg.to_string()
                );
                match message_sender.send(pending_message) {
                    Ok(_) => println!("[你 -> {}]: {}", target, msg),
                    Err(e) => eprintln!("发送消息失败: {}", e),
                }
            } else {
                println!("格式: @<用户名> <消息>");
            }
        } else {
            println!("格式: @<用户名> <消息>");
        }
    } else {
        let pending_message = P2PClient::create_chat_message_static(
            user_id.to_string(), 
            None, 
            input.to_string()
        );
        match message_sender.send(pending_message) {
            Ok(_) => println!("[你]: {}", input),
            Err(e) => eprintln!("发送消息失败: {}", e),
        }
    }
}