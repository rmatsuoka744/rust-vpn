#!/bin/bash

# デフォルト値の設定
DEFAULT_SERVER_BIND_ADDR="0.0.0.0"
DEFAULT_SERVER_PORT="12345"
DEFAULT_SERVER_TUN_IP="10.0.0.1/24"
DEFAULT_SERVER_TUN_NAME="tun0"

DEFAULT_CLIENT_SERVER_ADDR="127.0.0.1"
DEFAULT_CLIENT_PORT="12345"
DEFAULT_CLIENT_TUN_IP="10.0.0.2/24"
DEFAULT_CLIENT_TUN_NAME="tun1"

VPN_BINARY="./target/release/vpn"

# 使用法のヘルプ表示
usage() {
  echo "Usage: $0 [server | client] [options]"
  echo ""
  echo "Options:"
  echo "  --debug          Enable debug logging (default)"
  echo "  --info           Enable info-level logging only"
  echo ""
  echo "Server mode:"
  echo "  $0 server [bind_addr] [port] [tun_ip] [tun_name]"
  echo "    bind_addr: Address to bind the server (default: $DEFAULT_SERVER_BIND_ADDR)"
  echo "    port     : Port to listen on (default: $DEFAULT_SERVER_PORT)"
  echo "    tun_ip   : TUN interface IP (default: $DEFAULT_SERVER_TUN_IP)"
  echo "    tun_name : TUN interface name (default: $DEFAULT_SERVER_TUN_NAME)"
  echo ""
  echo "Client mode:"
  echo "  $0 client [server_addr] [port] [my_ip] [tun_name]"
  echo "    server_addr: Address of the server to connect to (default: $DEFAULT_CLIENT_SERVER_ADDR)"
  echo "    port       : Port to connect to (default: $DEFAULT_CLIENT_PORT)"
  echo "    my_ip      : TUN interface IP for the client (default: $DEFAULT_CLIENT_TUN_IP)"
  echo "    tun_name   : TUN interface name (default: $DEFAULT_CLIENT_TUN_NAME)"
  exit 1
}

# デフォルト値
LOG_LEVEL="debug"

# オプション解析
while [[ $# -gt 0 ]]; do
  case "$1" in
    server|client)
      MODE="$1"
      shift
      ;;
    --debug)
      LOG_LEVEL="debug"
      shift
      ;;
    --info)
      LOG_LEVEL="info"
      shift
      ;;
    *)
      if [[ -z "$MODE" ]]; then
        echo "[ERROR] You must specify either 'server' or 'client'."
        usage
      fi
      ARGS+=("$1")
      shift
      ;;
  esac
done

# 必要な引数が指定されていない場合はヘルプを表示
if [[ -z "$MODE" ]]; then
  echo "[ERROR] Missing mode (server or client)."
  usage
fi

# サーバーモードの処理
if [[ "$MODE" == "server" ]]; then
  BIND_ADDR=${ARGS[0]:-$DEFAULT_SERVER_BIND_ADDR}
  PORT=${ARGS[1]:-$DEFAULT_SERVER_PORT}
  TUN_IP=${ARGS[2]:-$DEFAULT_SERVER_TUN_IP}
  TUN_NAME=${ARGS[3]:-$DEFAULT_SERVER_TUN_NAME}

  echo "[INFO] Starting VPN server..."
  echo "[INFO] Bind address: $BIND_ADDR"
  echo "[INFO] Port: $PORT"
  echo "[INFO] TUN interface: $TUN_NAME"
  echo "[INFO] TUN IP: $TUN_IP"
  RUST_LOG="$LOG_LEVEL" "$VPN_BINARY" server "$BIND_ADDR" "$PORT" "$TUN_IP" "$TUN_NAME"

# クライアントモードの処理
elif [[ "$MODE" == "client" ]]; then
  SERVER_ADDR=${ARGS[0]:-$DEFAULT_CLIENT_SERVER_ADDR}
  PORT=${ARGS[1]:-$DEFAULT_CLIENT_PORT}
  MY_IP=${ARGS[2]:-$DEFAULT_CLIENT_TUN_IP}
  TUN_NAME=${ARGS[3]:-$DEFAULT_CLIENT_TUN_NAME}

  echo "[INFO] Starting VPN client..."
  echo "[INFO] Server address: $SERVER_ADDR"
  echo "[INFO] Port: $PORT"
  echo "[INFO] TUN interface: $TUN_NAME"
  echo "[INFO] TUN IP: $MY_IP"
  RUST_LOG="$LOG_LEVEL" "$VPN_BINARY" client "$SERVER_ADDR" "$PORT" "$MY_IP" "$TUN_NAME"

else
  echo "[ERROR] Invalid mode: $MODE"
  usage
fi
