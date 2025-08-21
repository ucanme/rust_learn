#!/bin/bash

# 编译项目
echo "Compiling project..."
cargo build
if [ $? -ne 0 ]; then
  echo "Compilation failed"
  exit 1
fi

# 启动服务器并记录PID
echo "Starting server..."
cargo run --bin epoll_server &
SERVER_PID=$!

echo "Server started with PID: $SERVER_PID"

# 等待服务器启动
sleep 2

# 运行客户端并捕获输出
echo "Running client..."
cargo run --bin epoll_client
CLIENT_EXIT_CODE=$?

# 停止服务器
echo "Stopping server..."
kill $SERVER_PID
wait $SERVER_PID 2>/dev/null

echo "Server stopped"

echo "Client exit code: $CLIENT_EXIT_CODE"
if [ $CLIENT_EXIT_CODE -eq 0 ]; then
  echo "Test passed: Client and server communication successful"
else
  echo "Test failed: Client exited with error code $CLIENT_EXIT_CODE"
fi

exit $CLIENT_EXIT_CODE