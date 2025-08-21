use p2p::common::Message;
use p2p::common::MessageType;
use std::time::SystemTime;

fn main() {
    println!("Testing p2p package import from workspace root...");
    
    // 创建一个简单的消息对象来测试导入
    let message = Message {
        msg_type: MessageType::Chat,
        sender_id: "test_user".to_string(),
        target_id: Some("other_user".to_string()),
        content: Some("Hello, world!".to_string()),
        sender_peer_address: "127.0.0.1".to_string(),
        sender_listen_port: 8081,
        timestamp: SystemTime::now(),
        sender_token: None,
    };
    
    println!("Created message: {:?}", message);
    println!("Import test successful!");
}