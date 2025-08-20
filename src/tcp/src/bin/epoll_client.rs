use mio::{Events, Interest, Poll, Token};
use mio::net::TcpStream;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::str;
use std::time::Duration;

const CLIENT: Token = Token(0);
const MAX_RETRY: u32 = 5;
const RETRY_DELAY: Duration = Duration::from_secs(1);

fn main() -> io::Result<()> {
    println!("EPOLL TCP Client starting...");
    
    // 解析地址
    let address: SocketAddr = match "127.0.0.1:18081".parse() {
        Ok(addr) => {
            println!("Parsed address: {}", addr);
            addr
        },
        Err(e) => {
            eprintln!("Failed to parse address: {}", e);
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Failed to parse address: {}", e)));
        }
    };

    // 连接服务器
    let mut stream = match connect_with_retry(&address) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to connect to server after {} attempts: {}", MAX_RETRY, e);
            return Err(e);
        }
    };
    println!("Successfully connected to server");

    // 创建poll实例
    let mut poll = match Poll::new() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to create Poll instance: {}", e);
            return Err(e);
        }
    };

    // 注册客户端流
    if let Err(e) = poll.registry().register(
        &mut stream,
        CLIENT,
        Interest::READABLE.add(Interest::WRITABLE),
    ) {
        eprintln!("Failed to register stream with poll: {}", e);
        return Err(e);
    }
    println!("Stream registered with poll");

    // 创建事件存储
    let mut events = Events::with_capacity(128);

    // 等待连接就绪（可写）
    println!("Waiting for connection to be ready...");
    poll.poll(&mut events, Some(Duration::from_secs(5)))?;
    
    let mut is_ready = false;
    for event in events.iter() {
        if event.token() == CLIENT && event.is_writable() {
            is_ready = true;
            break;
        }
    }
    
    if !is_ready {
        eprintln!("Connection not ready for writing within timeout");
        return Ok(());
    }

    // 发送消息
    let message = "Hello from EPOLL TCP client!";
    let mut retries = 3;
    let mut sent = false;
    
    while retries > 0 && !sent {
        match stream.write_all(message.as_bytes()) {
            Ok(()) => {
                println!("Sent: {}", message);
                sent = true;
                
                // 刷新缓冲区确保数据被发送
                if let Err(e) = stream.flush() {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        eprintln!("Flush would block, retrying...");
                        retries -= 1;
                        std::thread::sleep(Duration::from_millis(100));
                    } else {
                        eprintln!("Failed to flush stream: {}", e);
                        return Err(e);
                    }
                }
            },
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                eprintln!("Write would block, retrying...");
                retries -= 1;
                std::thread::sleep(Duration::from_millis(100));
            },
            Err(e) => {
                eprintln!("Failed to send message: {}", e);
                return Err(e);
            }
        }
    }
    
    if !sent {
        eprintln!("Failed to send message after multiple attempts");
        return Err(io::Error::new(io::ErrorKind::TimedOut, "Failed to send message"));
    }

    // 等待响应
    println!("Waiting for response...");
    if let Err(e) = poll.poll(&mut events, Some(Duration::from_secs(5))) {
        eprintln!("Poll error: {}", e);
        return Err(e);
    }

    if events.is_empty() {
        println!("Client timeout waiting for response");
        return Ok(());
    }

    println!("Received {} events", events.iter().count());
    for event in events.iter() {
        println!("Event: {:?}, token: {:?}", event, event.token());
        match event.token() {
            CLIENT => {
                if event.is_readable() {
                    println!("Client socket is readable");
                    let mut buffer = [0; 1024];
                    match stream.read(&mut buffer) {
                        Ok(0) => {
                            println!("Server closed connection");
                            // 从poll中注销流
                            if let Err(e) = poll.registry().deregister(&mut stream) {
                                eprintln!("Failed to deregister stream: {}", e);
                            }
                        },
                        Ok(n) => {
                            let received = str::from_utf8(&buffer[..n])
                                .unwrap_or("<invalid UTF-8>");
                            println!("Received: {}", received.trim_end());
                        },
                        Err(e) => {
                            eprintln!("Read error: {}", e);
                        }
                    }
                }
                if event.is_writable() {
                    println!("Client socket is writable");
                    // 我们已经发送了数据，这里不需要额外处理
                }
            },
            _ => {
                unreachable!()
            }
        }
    }

    Ok(())
}

// 带重试的连接函数
fn connect_with_retry(address: &SocketAddr) -> io::Result<TcpStream> {
    let mut retry_count = 0;
    loop {
        println!("Attempting to connect to {} (attempt {}/{})...", address, retry_count + 1, MAX_RETRY);
        match TcpStream::connect(*address) {
            Ok(stream) => {
                println!("Successfully connected to {}", address);
                return Ok(stream);
            },
            Err(e) => {
                if retry_count >= MAX_RETRY {
                    eprintln!("Maximum retries reached");
                    return Err(e);
                }
                eprintln!("Failed to connect: {}", e);
                retry_count += 1;
                std::thread::sleep(RETRY_DELAY);
            }
        }
    }
}