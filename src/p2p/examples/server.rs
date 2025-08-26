use p2p::server::P2PServer;
use p2p::common::P2PError;
use std::env;

fn main() -> Result<(), P2PError> {
    let addr = env::args().nth(1).unwrap_or_else(|| "127.0.0.1:8080".to_string());
    println!("Starting P2P server on {}...", addr);
    
    let mut server = P2PServer::new(&addr)?;
    println!("Server started successfully on {}!", addr);
    
    // Start the server event loop
    server.start()
}
