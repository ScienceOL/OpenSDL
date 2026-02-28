# OpenSDL + uni-lab-backend 集成测试指南

本文档是一份**完整的端到端测试指南**，帮助你在本地启动 OpenSDL 和 uni-lab-backend 并验证新的 gRPC 集成架构。

---

## 0. 当前分支状态

| 项目 | 分支 | 状态 |
|------|------|------|
| **OpenSDL** (`osdl/`) | `main` | gRPC 服务已实现并推送，可直接使用 |
| **uni-lab-backend** | `main` | OSDL 集成代码已在本地修改，**尚未创建 feat 分支** |

**首先需要为 uni-lab-backend 创建 feat 分支：**

```bash
cd /Users/Harvey/0/Code/osdl/uni-lab-backend

# 查看当前修改
git status

# 创建 feat 分支并提交
git checkout -b feat/osdl-integration
git add cmd/api/server.go internal/config/env.go \
        gen/ proto/ \
        pkg/core/schedule/dispatcher.go \
        pkg/core/schedule/dispatcher_osdl.go \
        pkg/core/schedule/dispatcher_redis.go \
        pkg/middleware/osdl/
git commit -m "feat: add OSDL gRPC integration with Dispatcher pattern"
git push -u origin feat/osdl-integration
```

---

## 1. 前提条件

### 必须安装

| 工具 | 版本 | 检查命令 |
|------|------|----------|
| Go | 1.24+ | `go version` |
| Docker | 20+ | `docker --version` |
| Docker Compose | v2+ | `docker compose version` |
| grpcurl | 最新 | `grpcurl --version` |

### 安装 grpcurl（如未安装）

```bash
# macOS
brew install grpcurl

# 或通过 Go 安装
go install github.com/fullstorydev/grpcurl/cmd/grpcurl@latest
```

---

## 2. 服务全景

测试环境包含以下服务：

```
┌─────────────────────────────────────────────────────────────────┐
│                       本地开发环境                               │
│                                                                 │
│  基础设施（Docker）:                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │ PostgreSQL   │  │ Redis        │  │ Casdoor（可选）       │  │
│  │ :5432        │  │ :6379        │  │ :8000                │  │
│  │ DB: osdl     │  │              │  │                      │  │
│  │ DB: studio   │  │              │  │                      │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
│                                                                 │
│  OpenSDL（本地或 Docker）:                                       │
│  ┌──────────────────────┐  ┌────────────────────────────────┐  │
│  │ API 服务              │  │ Schedule 服务                   │  │
│  │ HTTP :8080            │  │ WebSocket :8081                │  │
│  │ gRPC :9090            │  │ Redis 消费者                   │  │
│  └──────────────────────┘  └────────────────────────────────┘  │
│                                                                 │
│  uni-lab-backend（本地）:                                        │
│  ┌──────────────────────┐                                      │
│  │ API 服务              │                                      │
│  │ HTTP :80              │ ──gRPC──→ OpenSDL :9090             │
│  │ OSDL_ENABLED=true     │                                      │
│  └──────────────────────┘                                      │
└─────────────────────────────────────────────────────────────────┘
```

---

## 3. 启动基础设施

### 方式 A：使用 OSDL 的 Docker Compose（推荐）

```bash
cd /Users/Harvey/0/Code/osdl/osdl

# 启动 PostgreSQL + Redis
docker compose -p osdl-infra \
  -f docker/docker-compose.infra.yaml \
  --env-file docker/.env.dev up -d
```

验证基础设施：

```bash
# PostgreSQL
docker exec osdl-infra-postgres-1 pg_isready -U postgres
# 期望输出: /var/run/postgresql:5432 - accepting connections

# Redis
docker exec osdl-infra-redis-1 redis-cli ping
# 期望输出: PONG
```

### 方式 B：手动启动

```bash
# PostgreSQL（需要两个数据库）
docker run -d --name osdl-pg \
  -e POSTGRES_USER=postgres \
  -e POSTGRES_PASSWORD=osdl \
  -p 5432:5432 \
  postgres:16-alpine

# 等待 PG 就绪后创建两个数据库
docker exec osdl-pg psql -U postgres -c "CREATE DATABASE osdl;"
docker exec osdl-pg psql -U postgres -c "CREATE DATABASE studio;"

# Redis
docker run -d --name osdl-redis \
  -p 6379:6379 \
  redis:7-alpine
```

---

## 4. 启动 OpenSDL

### 4.1 配置环境变量

```bash
cd /Users/Harvey/0/Code/osdl/osdl

# 复制并编辑 .env
cp .env.example .env
```

编辑 `.env`，确认以下关键配置：

```bash
# 数据库
DATABASE_HOST=localhost
DATABASE_PORT=5432
DATABASE_NAME=osdl
DATABASE_USER=postgres
DATABASE_PASSWORD=osdl

# Redis
REDIS_HOST=127.0.0.1
REDIS_PORT=6379

# 服务端口
WEB_PORT=8080
SCHEDULE_PORT=8081
GRPC_PORT=9090
ENV=dev

# 认证（测试阶段可使用 casdoor，不影响 gRPC 测试）
OAUTH_SOURCE=casdoor
CASDOOR_ADDR=http://localhost:8000

# 日志
LOG_LEVEL=debug
LOG_PATH=./info.log
```

### 4.2 数据库迁移

```bash
go run . migrate
```

期望输出：

```
[INFO] Database migration completed
```

### 4.3 启动 API 服务（含 gRPC）

```bash
# 终端 1：API 服务
go run . apiserver
```

期望输出：

```
[INFO] gRPC server listening on :9090
[INFO] HTTP server listening on :8080
```

### 4.4 启动 Schedule 服务

```bash
# 终端 2：Schedule 服务
go run . schedule
```

期望输出：

```
[INFO] Schedule server listening on :8081
```

### 4.5（可选）使用 Air 热重载

```bash
# 终端 1
make dev

# 终端 2
make dev-schedule
```

---

## 5. 验证 OpenSDL gRPC 服务

### 5.1 列出所有 gRPC 服务

```bash
grpcurl -plaintext localhost:9090 list
```

期望输出：

```
grpc.reflection.v1.ServerReflection
grpc.reflection.v1alpha.ServerReflection
osdl.v1.AuthService
osdl.v1.EdgeService
osdl.v1.MaterialService
osdl.v1.ScheduleService
```

### 5.2 查看 ScheduleService 方法

```bash
grpcurl -plaintext localhost:9090 describe osdl.v1.ScheduleService
```

期望输出：

```
osdl.v1.ScheduleService is a service:
service ScheduleService {
  rpc StartAction ( .osdl.v1.StartActionRequest ) returns ( .osdl.v1.StartActionResponse );
  rpc StartNotebook ( .osdl.v1.StartNotebookRequest ) returns ( .osdl.v1.StartNotebookResponse );
  rpc StartWorkflow ( .osdl.v1.StartWorkflowRequest ) returns ( .osdl.v1.StartWorkflowResponse );
  rpc StopJob ( .osdl.v1.StopJobRequest ) returns ( .osdl.v1.StopJobResponse );
  rpc StreamJobStatus ( .osdl.v1.StreamJobStatusRequest ) returns ( stream .osdl.v1.JobStatusEvent );
}
```

### 5.3 测试 gRPC 认证拦截器

```bash
# 无 token 调用 — 应返回 Unauthenticated
grpcurl -plaintext \
  -d '{"lab_uuid":"test-lab-uuid"}' \
  localhost:9090 osdl.v1.EdgeService/GetEdgeStatus
```

期望输出：

```
ERROR:
  Code: Unauthenticated
  Message: missing authorization header
```

这证明认证拦截器正常工作。

### 5.4 查看请求/响应格式

```bash
# 查看 StartWorkflow 的请求格式
grpcurl -plaintext localhost:9090 describe osdl.v1.StartWorkflowRequest

# 查看 StartAction 的请求格式
grpcurl -plaintext localhost:9090 describe osdl.v1.StartActionRequest

# 查看 EdgeStatus 的请求格式
grpcurl -plaintext localhost:9090 describe osdl.v1.GetEdgeStatusRequest
```

---

## 6. 启动 uni-lab-backend（OSDL 模式）

### 6.1 切换到 feat 分支

```bash
cd /Users/Harvey/0/Code/osdl/uni-lab-backend
git checkout feat/osdl-integration
```

### 6.2 配置环境变量

创建或编辑 `.env`：

```bash
# 数据库（与 OSDL 共用同一 PostgreSQL 实例，不同数据库）
DATABASE_HOST=localhost
DATABASE_PORT=5432
DATABASE_NAME=studio
DATABASE_USER=postgres
DATABASE_PASSWORD=osdl

# Redis（与 OSDL 共用同一 Redis 实例）
REDIS_HOST=127.0.0.1
REDIS_PORT=6379
REDIS_PASSWORD=
REDIS_DB=0

# 服务
WEB_PORT=80
SCHEDULE_PORT=81
PLATFORM=sciol
SERVICE=api
ENV=dev

# ======== OSDL 集成（核心配置） ========
OSDL_ENABLED=true
OSDL_GRPC_ADDR=localhost:9090

# Nacos（uni-lab-backend 需要）
NACOS_ENDPOINT=127.0.0.1
NACOS_PORT=8848
NACOS_USER=nacos
NACOS_PASSWORD=nacos
NACOS_DATA_ID=studio-api
NACOS_GROUP=DEFAULT_GROUP
NACOS_NEED_WATCH=true
NACOS_NAMESPACE_ID=public

# 日志
LOG_PATH=./info.log
LOG_LEVEL=debug
```

> **注意**：uni-lab-backend 依赖 Nacos 做动态配置。如果你没有 Nacos，需要先启动一个：
>
> ```bash
> docker run -d --name nacos \
>   -e MODE=standalone \
>   -e SPRING_DATASOURCE_PLATFORM=embedded \
>   -p 8848:8848 \
>   nacos/nacos-server:v2.3.2
> ```

### 6.3 数据库迁移

```bash
go run main.go migrate
```

### 6.4 启动 uni-lab-backend API 服务

```bash
# 终端 3
go run main.go apiserver
```

关注启动日志中的关键输出：

```
OSDL gRPC dispatcher enabled, addr: localhost:9090
```

如果看到这行，说明 OSDL gRPC 集成已激活。

### 6.5 验证回退模式

将 `OSDL_ENABLED` 改为 `false` 重启，应看到：

```
Redis dispatcher enabled (OSDL disabled)
```

---

## 7. 端到端测试场景

### 测试 1：OSDL gRPC 连接验证

**目标**：确认 uni-lab-backend 能成功连接 OSDL gRPC。

**步骤**：
1. 确保 OSDL API 服务正在运行（端口 9090）
2. 启动 uni-lab-backend（`OSDL_ENABLED=true`）
3. 检查日志

**通过标准**：
- uni-lab-backend 日志出现 `OSDL gRPC dispatcher enabled, addr: localhost:9090`
- 无连接错误

**失败场景测试**：
1. 停止 OSDL API 服务
2. 重启 uni-lab-backend
3. 应看到 gRPC 连接错误日志，但服务仍应启动（连接是惰性的）

---

### 测试 2：StartAction（单步操作）

**目标**：验证从 uni-lab-backend 发起的 action 能通过 gRPC 到达 OSDL Redis 队列。

**步骤**：

1. 打开 Redis 监控（终端 4）：
   ```bash
   docker exec -it osdl-infra-redis-1 redis-cli MONITOR
   ```

2. 通过 grpcurl 直接调用 OSDL（绕过 uni-lab-backend 先测 OSDL 单独功能）：
   ```bash
   grpcurl -plaintext \
     -H "authorization: Bearer YOUR_TOKEN" \
     -d '{
       "lab_uuid": "00000000-0000-0000-0000-000000000001",
       "device_id": "pump-1",
       "action": "move",
       "action_type": "control",
       "param": "eyJ4IjoxMCwieSI6MjB9"
     }' \
     localhost:9090 osdl.v1.ScheduleService/StartAction
   ```
   > `param` 是 base64 编码的 JSON：`{"x":10,"y":20}`

3. 检查 Redis MONITOR 输出，应看到：
   ```
   "LPUSH" "lab_control_queue_00000000-0000-0000-0000-000000000001" "{...}"
   ```

**通过标准**：
- 返回 `task_uuid`
- Redis 中 `lab_control_queue_{lab_uuid}` 有新消息

---

### 测试 3：StartWorkflow（工作流调度）

**目标**：验证工作流任务正确推入 Redis 任务队列。

```bash
grpcurl -plaintext \
  -H "authorization: Bearer YOUR_TOKEN" \
  -d '{
    "lab_uuid": "00000000-0000-0000-0000-000000000001",
    "workflow_uuid": "wf-test-001",
    "user_id": "user-test-001"
  }' \
  localhost:9090 osdl.v1.ScheduleService/StartWorkflow
```

检查 Redis：

```bash
docker exec osdl-infra-redis-1 redis-cli \
  LRANGE lab_task_queue_00000000-0000-0000-0000-000000000001 0 -1
```

期望看到类似：

```json
{"action":"start_workflow","data":{"workflow_uuid":"wf-test-001","user_id":"user-test-001","task_uuid":"..."}}
```

---

### 测试 4：StopJob（停止任务）

```bash
grpcurl -plaintext \
  -H "authorization: Bearer YOUR_TOKEN" \
  -d '{
    "task_uuid": "上一步返回的task_uuid",
    "user_id": "user-test-001"
  }' \
  localhost:9090 osdl.v1.ScheduleService/StopJob
```

在 Redis MONITOR 中应看到 `PUBLISH` 命令到 `osdl:job:stop:{task_uuid}` 频道。

---

### 测试 5：GetEdgeStatus（设备状态）

```bash
grpcurl -plaintext \
  -H "authorization: Bearer YOUR_TOKEN" \
  -d '{"lab_uuid": "00000000-0000-0000-0000-000000000001"}' \
  localhost:9090 osdl.v1.EdgeService/GetEdgeStatus
```

期望输出（无 Edge 连接时）：

```json
{
  "is_online": false,
  "edge_session": "",
  "last_heartbeat": ""
}
```

---

### 测试 6：StreamJobStatus（流式状态推送）

**目标**：验证 Server-Side Streaming RPC。

1. 终端 A — 订阅状态流：
   ```bash
   grpcurl -plaintext \
     -H "authorization: Bearer YOUR_TOKEN" \
     -d '{"task_uuid": "test-task-001"}' \
     localhost:9090 osdl.v1.ScheduleService/StreamJobStatus
   ```

2. 终端 B — 模拟发布状态：
   ```bash
   docker exec osdl-infra-redis-1 redis-cli \
     PUBLISH osdl:job:status:test-task-001 \
     '{"status":"running","progress":50,"message":"Executing step 2/4"}'
   ```

3. 终端 A 应立即收到：
   ```json
   {
     "data": "{\"status\":\"running\",\"progress\":50,\"message\":\"Executing step 2/4\"}"
   }
   ```

---

### 测试 7：StreamDeviceStatus（设备状态流）

1. 终端 A — 订阅设备状态：
   ```bash
   grpcurl -plaintext \
     -H "authorization: Bearer YOUR_TOKEN" \
     -d '{"lab_uuid": "00000000-0000-0000-0000-000000000001"}' \
     localhost:9090 osdl.v1.EdgeService/StreamDeviceStatus
   ```

2. 终端 B — 模拟设备上报：
   ```bash
   docker exec osdl-infra-redis-1 redis-cli \
     PUBLISH osdl:device:status:00000000-0000-0000-0000-000000000001 \
     '{"device_id":"pump-1","status":"idle","temperature":25.3}'
   ```

3. 终端 A 应收到设备状态事件。

---

### 测试 8：Feature Flag 切换

**目标**：验证 `OSDL_ENABLED` 开关的行为一致性。

| 场景 | `OSDL_ENABLED` | 预期行为 |
|------|----------------|----------|
| A | `false` | 日志显示 `Redis dispatcher enabled`，任务直推 Redis |
| B | `true`，OSDL 在线 | 日志显示 `OSDL gRPC dispatcher enabled`，任务通过 gRPC 转发 |
| C | `true`，OSDL 离线 | 启动时 gRPC 连接警告，调度操作返回连接错误 |

---

## 8. Redis 队列验证

### 查看所有 OSDL 相关 Key

```bash
docker exec osdl-infra-redis-1 redis-cli KEYS '*lab*'
docker exec osdl-infra-redis-1 redis-cli KEYS 'osdl:*'
```

### 关键 Key 模式

| Key | 类型 | 说明 |
|-----|------|------|
| `lab_task_queue_{lab_uuid}` | List | 工作流 / 实验记录本任务队列 |
| `lab_control_queue_{lab_uuid}` | List | 动作 / 控制指令队列 |
| `lab_heart_key_{lab_uuid}` | String (TTL) | Edge 设备心跳（存在 = 在线） |
| `osdl:job:status:{task_uuid}` | Pub/Sub | 任务状态流式推送频道 |
| `osdl:job:stop:{task_uuid}` | Pub/Sub | 停止任务信号频道 |
| `osdl:device:status:{lab_uuid}` | Pub/Sub | 设备状态流式推送频道 |

### 实时监控所有 Redis 操作

```bash
docker exec osdl-infra-redis-1 redis-cli MONITOR
```

---

## 9. Swagger API 文档

OpenSDL 启动后，Swagger 文档可在浏览器访问：

```
http://localhost:8080/api/swagger/index.html
```

---

## 10. 常见问题排查

### gRPC 连接被拒绝

```
rpc error: code = Unavailable desc = connection error
```

**排查**：
1. 确认 OSDL API 服务正在运行：`lsof -i :9090`
2. 检查 `OSDL_GRPC_ADDR` 配置是否正确
3. 如果在 Docker 中运行，确认网络连通性

### 认证失败

```
rpc error: code = Unauthenticated desc = missing authorization header
```

**排查**：
1. 确保传递了 `authorization` metadata
2. Token 格式：`Bearer <access_token>` 或 `Lab <base64(key:secret)>`
3. 如果使用 Casdoor，确认 Casdoor 服务可达且 token 未过期

### 数据库连接失败

**排查**：
1. 确认 PostgreSQL 正在运行：`pg_isready -h localhost`
2. 确认数据库已创建：
   ```bash
   docker exec osdl-infra-postgres-1 psql -U postgres -l
   ```
3. 检查 `DATABASE_HOST`/`DATABASE_NAME` 配置

### Nacos 连接失败（uni-lab-backend）

uni-lab-backend 启动时需要 Nacos 做动态配置。如果 Nacos 不可用：

```bash
# 快速启动一个独立 Nacos
docker run -d --name nacos \
  -e MODE=standalone \
  -e SPRING_DATASOURCE_PLATFORM=embedded \
  -p 8848:8848 \
  nacos/nacos-server:v2.3.2
```

### Schedule 服务没有消费队列消息

**排查**：
1. 确认 Schedule 服务正在运行（端口 8081）
2. Schedule 只会消费**已连接 Edge 设备**对应的队列
3. 无 Edge 连接时，队列消息会堆积等待

---

## 11. 完整测试清单

按顺序执行，每一步标注 Pass/Fail：

| # | 测试项 | 命令/操作 | 预期结果 | 状态 |
|---|--------|----------|----------|------|
| 1 | 基础设施启动 | `docker compose up -d` | PostgreSQL + Redis 可达 | ☐ |
| 2 | OSDL 编译 | `cd osdl && go build ./...` | 无错误 | ☐ |
| 3 | OSDL 迁移 | `go run . migrate` | 数据库表已创建 | ☐ |
| 4 | OSDL API 启动 | `go run . apiserver` | 8080 + 9090 端口监听 | ☐ |
| 5 | OSDL Schedule 启动 | `go run . schedule` | 8081 端口监听 | ☐ |
| 6 | gRPC 服务列表 | `grpcurl -plaintext localhost:9090 list` | 4 个 service | ☐ |
| 7 | gRPC 认证拦截 | 无 token 调用任意 RPC | `Unauthenticated` | ☐ |
| 8 | StartAction | 调用 `StartAction` | 返回 `task_uuid` + Redis 有消息 | ☐ |
| 9 | StartWorkflow | 调用 `StartWorkflow` | 返回 `task_uuid` + Redis 有消息 | ☐ |
| 10 | StopJob | 调用 `StopJob` | Redis PUBLISH 到 stop 频道 | ☐ |
| 11 | GetEdgeStatus | 调用 `GetEdgeStatus` | 返回 `is_online: false` | ☐ |
| 12 | StreamJobStatus | 订阅 + 手动 PUBLISH | 收到流式事件 | ☐ |
| 13 | StreamDeviceStatus | 订阅 + 手动 PUBLISH | 收到流式事件 | ☐ |
| 14 | uni-lab 编译 | `cd uni-lab-backend && go build ./...` | 无错误 | ☐ |
| 15 | uni-lab OSDL=true | 启动并检查日志 | `OSDL gRPC dispatcher enabled` | ☐ |
| 16 | uni-lab OSDL=false | 启动并检查日志 | `Redis dispatcher enabled` | ☐ |
| 17 | Swagger | 访问 `localhost:8080/api/swagger/index.html` | 页面可加载 | ☐ |

---

## 12. 测试用 Token 获取

### 方式 A：Casdoor（如果已部署）

1. 访问 `http://localhost:8000`
2. 登录管理员账户
3. 进入 Applications → 获取 Client ID/Secret
4. 使用 OAuth2 授权码流获取 access_token：
   ```bash
   curl -X POST http://localhost:8000/api/login/oauth/access_token \
     -d "grant_type=authorization_code&client_id=YOUR_ID&client_secret=YOUR_SECRET&code=AUTH_CODE&redirect_uri=http://localhost:8080/api/auth/callback/casdoor"
   ```

### 方式 B：跳过认证测试（仅测试 gRPC 通道）

如果暂时不需要测试认证，可以在 OSDL 的 `pkg/grpc/interceptor.go` 中临时注释掉认证逻辑（**仅限开发环境**）。

### 方式 C：Lab 认证（Edge 设备模拟）

```bash
# 格式：Lab base64(AccessKey:AccessSecret)
# 示例 AccessKey=test-key, AccessSecret=test-secret
echo -n "test-key:test-secret" | base64
# 输出: dGVzdC1rZXk6dGVzdC1zZWNyZXQ=

grpcurl -plaintext \
  -H "authorization: Lab dGVzdC1rZXk6dGVzdC1zZWNyZXQ=" \
  -d '{"lab_uuid": "your-lab-uuid"}' \
  localhost:9090 osdl.v1.EdgeService/GetEdgeStatus
```

---

## 13. 清理环境

```bash
# 停止所有服务（Ctrl+C 各终端的 go run 进程）

# 停止并删除基础设施容器
cd /Users/Harvey/0/Code/osdl/osdl
docker compose -p osdl-infra -f docker/docker-compose.infra.yaml down -v

# 或停止手动启动的容器
docker rm -f osdl-pg osdl-redis nacos
```

---

## 附录：端口速查表

| 端口 | 服务 | 协议 |
|------|------|------|
| 5432 | PostgreSQL | TCP |
| 6379 | Redis | TCP |
| 8000 | Casdoor（可选） | HTTP |
| 8080 | OSDL API | HTTP |
| 8081 | OSDL Schedule | HTTP/WebSocket |
| 8848 | Nacos（uni-lab-backend 需要） | HTTP |
| 9090 | OSDL gRPC | gRPC |
| 80 | uni-lab-backend API | HTTP |
| 81 | uni-lab-backend Schedule | HTTP/WebSocket |
