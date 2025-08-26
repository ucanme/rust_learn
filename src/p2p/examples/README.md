# P2P 示例程序使用说明

本目录包含了P2P客户端和服务端的示例程序，用于演示如何使用p2p库进行点对点通信。

## 示例程序说明

### 服务端示例 (server.rs)
服务端示例程序演示了如何创建并启动P2P服务端，监听客户端连接请求。

### 客户端示例 (client.rs)
客户端示例程序演示了如何创建P2P客户端，连接到服务端，并提供了一个简单的命令行界面用于消息发送。

## 最新修复内容

✅ **已修复所有编译错误！**

### 修复的问题：
1. **client.rs中的逻辑错误** - 统一了服务器连接处理，修复了server_stream和streams混用的问题
2. **server.rs中的事件处理** - 解决了借用冲突问题，优化了事件循环逻辑
3. **消息处理和缓冲区管理** - 简化了消息序列化和发送逻辑，使用write_all确保完整发送
4. **错误处理优化** - 改进了错误处理机制，添加了更好的错误传播
5. **workspace配置** - 修复了Cargo.toml中的resolver版本问题

### 代码优化：
- 简化了事件处理循环，减少了代码复杂性
- 优化了消息序列化逻辑，提高了性能
- 为Message结构体添加了便利的构建方法
- **新增**：在client.rs中集成了优化的poll_once逻辑
- **新增**：添加了run_with_input_handler方法，简化客户端使用
- **重新设计**：将用户输入处理逻辑移回示例中，保持库的职责单一性
- **职责分离**：库专注网络通信，示例处理用户交互，设计更加合理
- **基于通道的优雅设计** 🎆：使用mpsc通道实现异步消息发送，更加优雅和高效
- **纯网络库设计** ✨：完全移除输入处理逻辑，库专注网络通信本身
- 改进了客户端示例程序的用户交互体验，支持中文界面

## P2P客户端 API 使用说明

### 主要方法

```rust
use p2p::client::{P2PClient, PendingMessage, MessageTarget};

// 1. 创建客户端
let mut client = P2PClient::new("127.0.0.1:8080", 0, "my_user_id".to_string())?;
client.connect()?;
client.request_peer_list()?;

// 2. 方式一：传统的直接调用方式
client.send_chat_message(None, "大家好！".to_string())?;
client.send_chat_message(Some("Alice".to_string()), "你好！".to_string())?;

// 3. 方式二：基于通道的优雅方式 🎆
let message_sender = client.get_message_sender();

// 在任何线程中发送消息！
let chat_message = client.create_chat_message(None, "大家好！".to_string());
message_sender.send(chat_message)?;

// 或者手动构建消息
let pending_message = PendingMessage {
    target: MessageTarget::Server,
    message: Message { /* ... */ },
};
message_sender.send(pending_message)?;

// 4. 运行客户端
client.run_with_input_handler(|client| {
    // 自定义输入处理逻辑
    Ok(true) // 返回true继续，false退出
})?;

// 或者手动事件处理（高级用法）
loop {
    client.poll_once()?; // 自动处理网络事件和待发送消息
}
```

### 优化特性

- **非阻塞事件处理**: `poll_once()` 方法允许高效的单次事件轮询
- **职责分离设计**: 库专注网络通信，示例处理用户交互，设计更加合理
- **基于通道的优雅消息发送** 🎆: 使用mpsc通道实现异步消息发送，解耦发送者和接收者
- **灵活的输入处理**: 示例程序可以自定义用户输入逻辑，不受库的限制
- **简化的运行模式**: `run_with_input_handler()` 集成了网络事件循环
- **线程安全**: 使用mpsc通道实现线程间通信，避免复杂的同步原语
- **模块化设计**: 库和示例的边界清晰，易于维护和扩展
- **简洁的API**: 提供直观易用的方法接口

### 示例程序的优雅设计

当前的客户端示例程序采用了最优雅的基于通道的设计：

**基于通道的消息发送架构** 🎆：
- 📱 **网络线程**：处理网络事件、接收消息、发送队列中的消息
- ⌨️ **输入线程**：专门处理用户输入，不阻塞网络操作
- 💬 **消息通道**：在线程间传递消息，完全解耦

```rust
// 获取消息发送器 - 可以在任何线程中使用！
let message_sender = client.get_message_sender();

// 主线程：处理网络事件和发送队列
client.run_with_input_handler(|client| { /* ... */ });

// 输入线程：优雅地发送消息
let chat_message = client.create_chat_message(None, input);
message_sender.send(chat_message).unwrap();
```

**这种设计的优势**：
- ✅ **真正的异步**: 消息发送不会阻塞网络事件处理
- ✅ **线程安全**: 使用Rust的mpsc通道，安全高效
- ✅ **易于测试**: 可以轻松模拟消息发送和网络事件
- ✅ **高性能**: 避免了不必要的等待和阻塞
- ✅ **灵活性**: 可以从任何线程发送消息（GUI、定时器等）
- ✅ **符合Rust理念**: 零成本抽象、所有权明确

这种设计真正体现了Rust中"零成本抽象"的理念！ 🚀

## 使用方法

### 运行示例

1. **启动服务端：**
```bash
cd /Users/ji.wu/RustroverProjects/learn/src/p2p
cargo run --example server
```

2. **在另一个终端中启动客户端：**
```bash
cd /Users/ji.wu/RustroverProjects/learn/src/p2p
cargo run --example client
```

3. **客户端使用方法：**
   - 启动后输入您的用户ID
   - 连接成功后，可以使用以下命令：
     - `<message>` - 发送公共消息
     - `@<username> <message>` - 发送私聊消息
     - `/exit` - 退出客户端

### 示例会话
```
Enter your user ID:
Alice
Connecting to server...
Connected to server!

Commands:
  @<username> <message>  - Send private message
  <message>              - Send public message
  /exit                  - Exit client

Hello everyone!
[You]: Hello everyone!
@Bob How are you?
[You -> Bob]: How are you?
/exit
Disconnecting...
```

## 架构特点

### 服务端架构
- 使用mio库实现高性能异步I/O
- 支持多客户端并发连接
- 消息路由和转发功能
- 心跳检测和连接超时处理
- 节点列表管理

### 客户端架构  
- 异步事件驱动设计
- 支持公共和私聊消息
- 自动重连机制(计划中)
- 简洁的命令行界面

### 消息类型支持
- Join: 客户端加入
- Leave: 客户端离开
- Chat: 聊天消息
- PeerList: 节点列表
- Heartbeat: 心跳检测
- ConnectRequest/Response: 连接请求响应

## 开发说明

1. ✅ 修复了所有编译错误
2. ✅ 优化了事件处理机制
3. ✅ 改进了错误处理
4. ✅ 简化了代码逻辑
5. ✅ 提供了完整的示例程序

## 下一步计划

1. 添加自动重连功能
2. 实现文件传输功能
3. 添加安全认证机制
4. 支持群组聊天
5. 添加更多的单元测试