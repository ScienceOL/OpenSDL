#!/bin/bash

# =============================================
# OSDL 开发环境控制脚本
# =============================================

# -------------------------------
# 全局配置
# -------------------------------
SCRIPT_DIR=$(dirname "$0")
PROJECT_DIR=$(dirname "${SCRIPT_DIR}")
ENV_FILE="${PROJECT_DIR}/docker/.env.dev"

# -------------------------------
# 颜色配置
# -------------------------------
source "${SCRIPT_DIR}/colors.sh"

# -------------------------------
# 帮助信息
# -------------------------------
print_help() {
  echo -e "${BRIGHT_GREEN}OSDL Dev Environment${RESET}"
  echo -e "  dev.sh [options]"
  echo
  echo -e "${BRIGHT_GREEN}Options:${RESET}"
  echo -e "  ${YELLOW}-h${RESET}   Show help"
  echo -e "  ${YELLOW}-d${RESET}   Start in daemon mode (background)"
  echo -e "  ${YELLOW}-e${RESET}   Stop and remove all containers"
  echo -e "  ${YELLOW}-s${RESET}   Stop containers (keep volumes)"
  echo
  echo -e "${BRIGHT_GREEN}Examples:${RESET}"
  echo -e "  $ ./dev.sh        # Start in foreground"
  echo -e "  $ ./dev.sh -d     # Start in background"
  echo -e "  $ ./dev.sh -s     # Stop containers"
  echo -e "  $ ./dev.sh -e     # Remove containers"
  exit 0
}

# -------------------------------
# 环境检查
# -------------------------------
check_basics() {
  echo -e "${BRIGHT_MAGENTA}\n[1/2] Checking prerequisites...${RESET}"

  if ! command -v docker &> /dev/null; then
    echo -e "${BRIGHT_RED}Error: Docker not found${RESET}"
    exit 1
  fi

  if [ ! -f "${ENV_FILE}" ]; then
    echo -e "${BRIGHT_YELLOW}Creating .env.dev from defaults...${RESET}"
    cat > "${ENV_FILE}" << 'ENVEOF'
DATABASE_HOST=host.docker.internal
DATABASE_PORT=5432
DATABASE_NAME=osdl
DATABASE_USER=postgres
DATABASE_PASSWORD=osdl
REDIS_HOST=host.docker.internal
REDIS_PORT=6379
ENV=dev
LOG_LEVEL=debug
OAUTH_SOURCE=casdoor
CASDOOR_ADDR=http://host.docker.internal:8000
ENVEOF
  fi

  echo -e "${BRIGHT_GREEN}Docker and .env ready.${RESET}"
}

# -------------------------------
# 安装 swag 工具
# -------------------------------
check_swag() {
  echo -e "${BRIGHT_MAGENTA}\n[2/2] Checking dev tools...${RESET}"

  if ! command -v swag &> /dev/null; then
    echo -e "${BRIGHT_YELLOW}Installing swag (Swagger generator)...${RESET}"
    go install github.com/swaggo/swag/cmd/swag@latest
  fi

  if ! command -v air &> /dev/null; then
    echo -e "${BRIGHT_YELLOW}Installing air (hot-reload)...${RESET}"
    go install github.com/air-verse/air@v1.62.0
  fi

  echo -e "${BRIGHT_GREEN}Dev tools ready.${RESET}"
}

# =============================================
# 参数解析
# =============================================
BACKGROUND_MODE=0
EXIT_COMMAND=0
STOP_COMMAND=0

while getopts "hdes" opt; do
  case $opt in
    e) EXIT_COMMAND=1 ;;
    h) print_help ;;
    d) BACKGROUND_MODE=1 ;;
    s) STOP_COMMAND=1 ;;
    \?)
      echo -e "${BRIGHT_RED}Invalid option: -$OPTARG${RESET}" >&2
      exit 1
      ;;
  esac
done

# =============================================
# 主执行流程
# =============================================

echo -e "${BRIGHT_BLUE}\n  OSDL — Open Self-Driving Lab${RESET}"
echo -e "${DIM}  Edge device communication infrastructure${RESET}\n"

check_basics
check_swag

# Docker Compose 参数 — 开发服务
CMD_ARGS=(
  -f "${PROJECT_DIR}/docker/docker-compose.base.yaml"
  -f "${PROJECT_DIR}/docker/docker-compose.dev.yaml"
  --env-file "${ENV_FILE}"
)

# Docker Compose 参数 — 基础设施
MID_CMD_ARGS=(
  -p "osdl-infra"
  -f "${PROJECT_DIR}/docker/docker-compose.infra.yaml"
  --env-file "${ENV_FILE}"
)

# 处理关闭
if [ "${EXIT_COMMAND}" -eq 1 ]; then
  echo -e "${BRIGHT_YELLOW}Stopping and removing dev containers...${RESET}"
  docker compose "${CMD_ARGS[@]}" down
  echo -e "${BRIGHT_YELLOW}Stopping and removing infra containers...${RESET}"
  docker compose "${MID_CMD_ARGS[@]}" down
  exit
fi

# 处理停止
if [ "${STOP_COMMAND}" -eq 1 ]; then
  echo -e "${BRIGHT_YELLOW}Stopping dev containers...${RESET}"
  docker compose "${CMD_ARGS[@]}" stop
  echo -e "${BRIGHT_YELLOW}Stopping infra containers...${RESET}"
  docker compose "${MID_CMD_ARGS[@]}" stop
  exit
fi

# 检查基础设施状态
echo -e "${BRIGHT_CYAN}\nChecking infrastructure services...${RESET}"
RUNNING_MID_SERVICES=$(docker compose "${MID_CMD_ARGS[@]}" ps --status=running -q 2>/dev/null)
if [ -n "$RUNNING_MID_SERVICES" ]; then
  echo -e "${BRIGHT_GREEN}Infrastructure services already running.${RESET}"
else
  echo -e "${BRIGHT_YELLOW}Starting infrastructure services...${RESET}"
  docker compose "${MID_CMD_ARGS[@]}" up -d
  if [ $? -ne 0 ]; then
    echo -e "${BRIGHT_RED}Failed to start infrastructure.${RESET}"
    exit 1
  fi
  echo -e "${BRIGHT_GREEN}Infrastructure started.${RESET}"
fi

# 启动开发服务
echo -e "${BRIGHT_BLUE}\nStarting OSDL dev services...${RESET}"
if [ "${BACKGROUND_MODE}" -eq 1 ]; then
  echo -e "${BRIGHT_YELLOW}Running in daemon mode${RESET}"
  docker compose "${CMD_ARGS[@]}" up -d
else
  echo -e "${BRIGHT_YELLOW}Running in foreground (Ctrl+C to stop)${RESET}"
  docker compose "${CMD_ARGS[@]}" up
fi
