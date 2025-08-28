#!/bin/bash

echo "🌐 多客户端P2P测试脚本"
echo "=========================="

# 检查服务器是否在运行
if ! lsof -i:8080 > /dev/null 2>&1; then
    echo "🚀 启动P2P服务器..."
    cd /Users/ji.wu/RustroverProjects/learn/src/p2p
    cargo run --example server &
    SERVER_PID=$!
    echo "服务器PID: $SERVER_PID"
    sleep 2
else
    echo "✅ 服务器已在运行"
fi

echo ""
echo "📋 可用的测试方法："
echo ""
echo "方法1: 手动测试（推荐）"
echo "----------------------"
echo "在不同终端窗口中运行以下命令："
echo ""
echo "终端A: cargo run --example client  # 输入: alice"
echo "终端B: cargo run --example client  # 输入: bob" 
echo "终端C: cargo run --example client  # 输入: charlie"
echo ""
echo "方法2: 快速启动多个客户端"
echo "------------------------"
echo "运行: ./multi_client_test.sh start"
echo ""
echo "测试命令序列："
echo "1. 在alice中: /list"
echo "2. 在alice中: /p2p bob"
echo "3. 在alice中: /direct bob hello"
echo "4. 在bob中: /direct alice hi"
echo ""

if [ "$1" = "start" ]; then
    echo "🎯 启动测试客户端..."
    
    # 启动alice客户端（后台）
    echo "alice" | timeout 30 cargo run --example client > alice.log 2>&1 &
    ALICE_PID=$!
    echo "✅ Alice客户端启动 (PID: $ALICE_PID)"
    
    # 启动bob客户端（后台）  
    echo "bob" | timeout 30 cargo run --example client > bob.log 2>&1 &
    BOB_PID=$!
    echo "✅ Bob客户端启动 (PID: $BOB_PID)"
    
    sleep 2
    
    echo ""
    echo "📊 当前运行的进程："
    echo "服务器: PID $SERVER_PID"
    echo "Alice: PID $ALICE_PID (日志: alice.log)"
    echo "Bob: PID $BOB_PID (日志: bob.log)"
    echo ""
    echo "💡 查看日志: tail -f alice.log 或 tail -f bob.log"
    echo "🛑 停止测试: ./multi_client_test.sh stop"
    
elif [ "$1" = "stop" ]; then
    echo "🛑 停止所有测试进程..."
    pkill -f "cargo run --example client" || true
    pkill -f "cargo run --example server" || true
    rm -f alice.log bob.log charlie.log
    echo "✅ 所有进程已停止"
    
elif [ "$1" = "logs" ]; then
    echo "📊 实时查看日志..."
    if [ -f alice.log ] && [ -f bob.log ]; then
        echo "=== Alice 日志 ==="
        tail -n 10 alice.log
        echo ""
        echo "=== Bob 日志 ==="
        tail -n 10 bob.log
    else
        echo "❌ 日志文件不存在，请先运行测试"
    fi
fi

echo ""
echo "💡 使用说明："
echo "  ./multi_client_test.sh        # 显示此帮助"
echo "  ./multi_client_test.sh start  # 启动多个客户端"
echo "  ./multi_client_test.sh stop   # 停止所有客户端"  
echo "  ./multi_client_test.sh logs   # 查看日志"