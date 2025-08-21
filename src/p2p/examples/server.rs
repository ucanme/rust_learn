use p2p::server::P2PServer;
use p2p::common::P2PError;

fn main() -> Result<(), P2PError> {
    println!("Starting P2P server...");
    
    // 创建并启动P2P服务端，监听8888端口
    let mut server = P2PServer::new("0.0.0.0:8888")?;
    println!("P2P server started successfully");
    
    // 启动服务端事件循环
    // 这个调用会一直阻塞直到服务端关闭
    server.start()
}