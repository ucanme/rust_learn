use p2p::client::{P2PClient, PendingMessage};
use p2p::common::P2PError;
use std::io::{self, BufRead};
use std::env;
use std::thread;
use std::time::Duration;
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
    
    // 创建、连接P2P客户端
    let mut client = P2PClient::new(&server_addr, 0, user_id.clone())?;
    client.connect()?;
    client.request_peer_list()?;
    
    println!("已连接到服务器！用户: {}", user_id);
    println!("\n使用说明:");
    println!("  直接输入消息发送公共消息");
    println!("  @<用户名> <消息> 发送私聊消息");
    println!("  /exit 退出客户端\n");
    
    // 获取消息发送器——这是新的优雅方式！
    let message_sender = client.get_message_sender();
    
    // 创建用于接收用户输入的通道
    let (input_tx, input_rx) = mpsc::channel::<String>();
    
    // 在单独线程中处理用户输入
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut handle = stdin.lock();
        
        loop {
            let mut input = String::new();
            match handle.read_line(&mut input) {
                Ok(_) => {
                    let input = input.trim().to_string();
                    if input_tx.send(input).is_err() {
                        break; // 主线程已经退出
                    }
                }
                Err(_) => break,
            }
        }
    });
    
    // 使用库的run_with_input_handler方法
    client.run_with_input_handler(|client_ref| {
        // 检查用户输入
        match input_rx.try_recv() {
            Ok(input) => {
                if input.is_empty() {
                    return Ok(true); // 继续运行
                }
                
                if input.eq_ignore_ascii_case("/exit") {
                    return Ok(false); // 退出
                }
                
                // 使用新的基于通道的消息发送方式
                handle_user_input_with_channel(client_ref, &input, &message_sender)?;
            }
            Err(mpsc::TryRecvError::Empty) => {
                // 没有输入，继续
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                return Ok(false); // 输入线程已经结束
            }
        }
        
        // 短暂休眠避免占用过多CPU
        thread::sleep(Duration::from_millis(50));
        Ok(true) // 继续运行
    })?;
    
    println!("客户端已断开连接。");
    Ok(())
}

/// 在示例中处理用户输入的函数（基于通道的新方式）
fn handle_user_input_with_channel(
    client: &P2PClient, 
    input: &str, 
    message_sender: &mpsc::Sender<PendingMessage>
) -> Result<(), P2PError> {
    // 处理消息发送 - 使用通道的优雅方式
    if let Some(message) = input.strip_prefix('@') {
        if let Some((target, msg)) = message.split_once(' ') {
            let target = target.trim();
            let msg = msg.trim();
            if !target.is_empty() && !msg.is_empty() {
                let pending_message = client.create_chat_message(Some(target.to_string()), msg.to_string());
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
        let pending_message = client.create_chat_message(None, input.to_string());
        match message_sender.send(pending_message) {
            Ok(_) => println!("[你]: {}", input),
            Err(e) => eprintln!("发送消息失败: {}", e),
        }
    }
    Ok(())
}