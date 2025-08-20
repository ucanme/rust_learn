use mio::{Events, Interest, Poll, Token};
use mio::net::{TcpListener, TcpStream};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::str;

// 定义token常量
const SERVER: Token = Token(0);
const MAX_CONN: usize = 1024;

fn main() -> io::Result<()> {
    // 创建poll实例
    let mut poll = Poll::new()?;
    // 创建事件存储
    let mut events = Events::with_capacity(MAX_CONN);

    // 绑定TCP监听
    let addr: SocketAddr = match "127.0.0.1:18081".parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Failed to parse address: {}", e);
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("Failed to parse address: {}", e)));
        }
    };
    let mut server = match TcpListener::bind(addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Failed to bind to address {}: {}", addr, e);
            return Err(e);
        }
    };

    // 注册服务端socket
    poll.registry().register(
        &mut server,
        SERVER,
        Interest::READABLE,
    )?;

    // 存储客户端连接
    let mut connections = HashMap::new();
    let mut next_token = Token(1);

    println!("EPOLL TCP Server running on 127.0.0.1:8081...");

    // 事件循环
    loop {
        // 等待事件
        poll.poll(&mut events, None)?;

        for event in events.iter() {
            match event.token() {
                SERVER => loop {
                    // 接受新连接
                    match server.accept() {
                        Ok((mut stream, addr)) => {
                            println!("New connection: {}", addr);

                            // 为新连接分配token
                            let token = next_token;
                            next_token = Token(token.0 + 1);

                            // 注册新连接
                            poll.registry().register(
                                &mut stream,
                                token,
                                Interest::READABLE.add(Interest::WRITABLE),
                            )?;

                            // 存储连接
                            connections.insert(token, stream);
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            break; // 没有更多连接
                        }
                        Err(e) => {
                            eprintln!("Accept error: {}", e);
                            break;
                        }
                    }
                },
                token => {
                            // 处理客户端连接事件
                            // 标记是否需要移除连接
                            let mut should_remove = false;

                            if let Some(mut stream) = connections.get_mut(&token) {
                                if event.is_readable() {
                                    // 读取数据
                                    let mut buffer = [0; 1024];
                                    match stream.read(&mut buffer) {
                                        Ok(0) => {
                                            // 客户端关闭连接
                                            println!("Client disconnected");
                                            should_remove = true;
                                        }
                                        Ok(n) => {
                                            let received = str::from_utf8(&buffer[..n])
                                                .unwrap_or("<invalid UTF-8>");
                                            println!("Received: {}", received.trim_end());

                                            // 回显数据
                                            // 尝试写入数据
                                            let mut buf: Vec<u8>= "server reply ".as_bytes().to_vec();
                                            buf.append(&mut buffer.to_vec());

                                            match stream.write_all(&buf[..buf.len()]) {
                                                Ok(()) => {
                                                    println!("Sent: {}", received.trim_end());
                                                }
                                                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                                    // 流暂时不可写，实际应用中应实现数据缓存机制
                                                    eprintln!("Stream not writable, would block");
                                                    // 不立即移除连接，而是等待下次可写事件
                                                }
                                                Err(e) => {
                                                    eprintln!("Write error: {}", e);
                                                    should_remove = true;
                                                }
                                            }
                                            
                                            // 确保数据被刷新
                                            if let Err(e) = stream.flush() {
                                                if e.kind() == io::ErrorKind::WouldBlock {
                                                    // 刷新操作也可能阻塞
                                                    eprintln!("Flush would block, will retry later");
                                                } else {
                                                    eprintln!("Flush error: {}", e);
                                                    should_remove = true;
                                                }
                                            }
                                        }
                                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                                            continue;
                                        }
                                        Err(e) => {
                                            eprintln!("Read error: {}", e);
                                            should_remove = true;
                                        }
                                    }
                                }

                                if event.is_writable() {
                                    // 这里可以处理写入事件（如果需要）
                                    // 对于简单的回显服务器，我们不需要特别处理可写事件
                                }
                            }

                            // 在可变引用作用域之外执行移除操作
                            if should_remove {
                                connections.remove(&token);
                            }
                }
            }
        }
    }
}