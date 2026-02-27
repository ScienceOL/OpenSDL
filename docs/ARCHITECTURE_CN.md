# OSDL 系统架构 — 完整功能划分与数据流转

本文档提供 **OSDL + uni-lab-backend + Edge** 三层架构的完整视图，涵盖功能边界、数据归属、集成点和运行时数据流转。

> **语言 / Language**: 中文 | [English](./ARCHITECTURE.md)

---

## 1. 系统总览

完整的科学实验室平台由三个独立部署组成：

| 组件 | 角色 | 端口 | 技术栈 |
|------|------|------|--------|
| **uni-lab-backend** | 业务逻辑、用户管理、模板、审批 | HTTP :80 | Go + Gin + GORM + PostgreSQL |
| **OSDL API 服务** | Edge 通信网关、gRPC API、OAuth2 | HTTP :8080 + gRPC :9090 | Go + Gin + gRPC + Redis |
| **OSDL Schedule 服务** | WebSocket 中心、任务执行引擎 | WS :8081 | Go + Melody + Redis |
| **Edge 设备** | 物理实验室仪器 (Uni-Lab-Edge / unilabos) | — | Python / C++ |

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        浏览器 / 客户端 / Studio                              │
└──────┬──────────────────────────────┬───────────────────────────────────────┘
       │ HTTP :80                     │ HTTP :8080 / gRPC :9090
       ▼                              ▼
┌──────────────────┐           ┌──────────────────────────┐
│ uni-lab-backend   │──gRPC───→│    OSDL API 服务          │
│                   │          │                          │
│ 用户 / 工作流     │          │  物料 CRUD               │
│ 实验本 / RBAC     │          │  OAuth2 (Casdoor/Bohr)  │
│ 审批 / 存储       │          │  gRPC 服务 (×4)          │
│ 试剂 / OPA        │          │  SSE 通知               │
│ Nacos / 模板      │          │  Swagger UI              │
└──────────────────┘           └────────────┬─────────────┘
                                            │ Redis 队列 + Pub/Sub
                                            ▼
                               ┌──────────────────────────┐
                               │  OSDL Schedule 服务       │
                               │                          │
                               │  WebSocket 中心 (Melody) │
                               │  任务队列消费者           │
                               │  控制队列消费者           │
                               │  引擎: DAG / Notebook    │
                               │        / Action          │
                               └────────────┬─────────────┘
                                            │ WebSocket :8081
                                            ▼
                               ┌──────────────────────────┐
                               │  Edge 设备                │
                               │ (Uni-Lab-Edge / unilabos) │
                               └──────────────────────────┘
```

---

## 2. Mermaid 架构图

```mermaid
graph TB
    subgraph Client["浏览器 / Studio"]
        WEB[Web 界面]
        SDK[客户端 SDK]
    end

    subgraph ULB["uni-lab-backend :80"]
        direction TB
        ULB_API[REST API]
        ULB_AUTH[用户认证]
        subgraph ULB_BIZ["业务逻辑层"]
            ENV[实验室 / 用户 / 组织]
            WF[工作流模板]
            NB[实验本模板]
            RBAC[RBAC / OPA 策略]
            APPR[审批工作流]
            REAG[试剂管理]
            STOR[文件存储 / OSS]
        end
        DISP{调度分发器<br/>OSDL_ENABLED?}
    end

    subgraph OSDL["OSDL 平台"]
        subgraph OSDL_API["API 服务 :8080 / :9090"]
            HTTP_API[HTTP REST + Swagger]
            GRPC_SVC["gRPC 服务<br/>Schedule / Material<br/>Edge / Auth"]
            OAUTH["OAuth2<br/>(Casdoor / Bohrium)"]
            MAT_CRUD[物料 CRUD]
            SSE[SSE 通知]
            INTERCEPTOR[认证拦截器]
        end
        subgraph OSDL_SCH["Schedule 服务 :8081"]
            WS_HUB["WebSocket 中心<br/>(Melody)"]
            TQ["任务队列<br/>(Redis BRPop)"]
            CQ["控制队列<br/>(Redis BRPop)"]
            ENGINE["执行引擎<br/>DAG / Notebook / Action"]
            HEARTBEAT["心跳监控"]
        end
    end

    subgraph Infra["基础设施"]
        PG[(PostgreSQL)]
        RD[(Redis)]
        CAS["Casdoor :8000"]
    end

    subgraph Edge["Edge 设备"]
        E1["设备 1<br/>(泵、反应器...)"]
        E2["设备 2"]
        EN["设备 N"]
    end

    WEB & SDK -->|HTTP| ULB_API
    WEB & SDK -->|HTTP / gRPC| OSDL_API
    ULB_API --> ULB_AUTH --> ULB_BIZ
    ULB_BIZ --> DISP
    DISP -->|"OSDL_ENABLED=true<br/>gRPC :9090"| GRPC_SVC
    DISP -->|"OSDL_ENABLED=false<br/>Redis LPush"| RD
    INTERCEPTOR --> GRPC_SVC
    GRPC_SVC --> RD
    HTTP_API --> MAT_CRUD
    MAT_CRUD --> PG
    OAUTH --> CAS
    RD --> TQ & CQ
    TQ & CQ --> ENGINE
    ENGINE --> WS_HUB
    E1 & E2 & EN <-->|WebSocket| WS_HUB
    HEARTBEAT -->|SetEx TTL| RD
    ULB_BIZ --> PG
    SSE -.->|Redis Pub/Sub| RD

    style OSDL fill:#e8f5e9
    style ULB fill:#e3f2fd
    style Edge fill:#fff3e0
```

---

## 3. 完整功能划分

### 3.1 uni-lab-backend（75% — 业务逻辑层）

| 领域 | 模块 | 核心操作 | 数据存储 |
|------|------|----------|----------|
| **用户 / 组织** | `core/environment/` | 用户管理、实验室 CRUD、成员邀请/移除、实验室置顶 | PostgreSQL |
| **权限控制** | `core/inner/` + OPA | RBAC 角色、角色权限、用户角色绑定、自定义策略 | PostgreSQL + OPA |
| **工作流模板** | `core/workflow/` | 模板 CRUD、Fork、导入/导出、版本管理、标签分类 | PostgreSQL |
| **工作流节点模板** | `core/workflow/` | 节点模板 CRUD、Schema 定义、设备能力查询 | PostgreSQL |
| **实验本模板** | `core/notebook/` | 实验本 CRUD、样本追踪、Schema 定义 | PostgreSQL |
| **物料定义** | `core/material/` (UI) | 在 Web 界面创建物料图、模板管理 | PostgreSQL |
| **试剂管理** | `core/reagent/` | 试剂 CRUD、CAS 号查询 (PubChem API) | PostgreSQL |
| **审批工作流** | `core/sse/` | 工作流提交审批、审批链定义、批准/拒绝 | PostgreSQL |
| **文件存储** | `core/storage/` | 预签名 URL 生成、OSS 集成 | OSS (S3) |
| **通知系统** | `core/notify/` + `core/sse/` | SSE 流、通知 CRUD、跨 Pod 广播 | Redis Pub/Sub |
| **动态配置** | Nacos | 热加载配置、功能开关 | Nacos |
| **任务分发** | `core/schedule/` | 通过 `Dispatcher` 接口分发工作流/实验本/Action | — |
| **任务历史** | `web/views/workflow/` | 任务列表、任务下载、状态查询 | PostgreSQL |

### 3.2 OSDL（25% — Edge 通信与调度层）

| 领域 | 模块 | 核心操作 | 数据存储 |
|------|------|----------|----------|
| **Edge 连接** | `core/schedule/control/` | WebSocket 会话生命周期 (Melody)、200 连接池 | 内存 |
| **Edge 认证** | `middleware/auth/` | Lab AK/SK 头部验证 (`Lab base64(AK:SK)`) | PostgreSQL |
| **心跳监控** | `core/schedule/lab/edge/` | 定期 `SetEx` 到 `lab_heart_key_{lab_uuid}` (TTL 1000s) | Redis |
| **任务执行** | `core/schedule/engine/` | DAG 工作流、实验本、单步 Action 执行 | Redis |
| **队列消费** | `core/schedule/lab/edge/` | BRPop 消费 `lab_task_queue_*` 和 `lab_control_queue_*` | Redis |
| **物料运行时** | `core/material/` | Edge 上报物料创建/更新、设备状态同步 | PostgreSQL |
| **设备状态** | `core/schedule/lab/edge/` | 设备属性更新通过 Redis Pub/Sub 广播 | Redis Pub/Sub |
| **OAuth2** | `core/login/casdoor/` + `repo/bohr/` | Casdoor OAuth2 + Bohrium JWT，通过 `OAUTH_SOURCE` 切换 | Redis (state) |
| **gRPC API** | `pkg/grpc/services/` | 4 个服务、14 个 RPC 供上游集成 | Redis + PostgreSQL |
| **HTTP API** | `pkg/web/` | 物料 CRUD、健康检查、SSE、Swagger UI | PostgreSQL |

### 3.3 Edge 设备（物理层）

| 领域 | 操作 |
|------|------|
| **设备控制** | 执行动作（泵、加热、搅拌、测量...） |
| **状态上报** | 通过 WebSocket 推送 device_status 消息 |
| **物料拓扑** | 上报物理设备图（POST 到 OSDL Edge API） |
| **任务执行** | 接收动作指令，通过 job_status 上报完成 |
| **心跳保活** | 定期 ping/pong 维持连接活跃 |

---

## 4. 数据归属

```mermaid
graph LR
    subgraph PostgreSQL["PostgreSQL（持久化）"]
        direction TB
        ULB_DATA["uni-lab-backend 拥有：<br/>• 用户、组织<br/>• 工作流、实验本<br/>• 审批、试剂<br/>• 角色、权限<br/>• 存储令牌、标签"]
        OSDL_DATA["OSDL 拥有：<br/>• material_nodes<br/>• material_edges<br/>• 实验室环境<br/>• 沙箱配置"]
    end

    subgraph Redis["Redis（运行时）"]
        direction TB
        QUEUES["任务队列：<br/>• lab_task_queue_{uuid}<br/>• lab_control_queue_{uuid}"]
        PUBSUB["Pub/Sub 频道：<br/>• osdl:job:status:{uuid}<br/>• osdl:job:stop:{uuid}<br/>• osdl:device:status:{uuid}"]
        HEARTBEAT_KEY["心跳键：<br/>• lab_heart_key_{uuid}<br/>（TTL 过期检测）"]
        OAUTH_STATE["OAuth 状态：<br/>• state → callback_url<br/>（5 分钟过期）"]
    end

    subgraph Memory["内存（易失）"]
        WS_SESSIONS["WebSocket 会话：<br/>• 每个实验室的 EdgeImpl<br/>• Melody 会话池"]
        TASK_INSTANCES["任务实例：<br/>• 运行中的 DAG 引擎<br/>• 运行中的 Notebook 引擎<br/>• 运行中的 Action 任务"]
    end
```

---

## 5. 时序图

### 5.1 工作流执行（完整路径）

```mermaid
sequenceDiagram
    participant User as 浏览器
    participant ULB as uni-lab-backend :80
    participant OSDL_API as OSDL API :9090
    participant Redis as Redis
    participant OSDL_SCH as OSDL Schedule :8081
    participant Edge as Edge 设备

    Note over User,ULB: 1. 用户触发工作流执行

    User->>ULB: PUT /api/v1/lab/run/workflow<br/>{workflow_uuid, lab_uuid}
    ULB->>ULB: 验证工作流模板<br/>检查审批状态<br/>解析设备绑定

    alt OSDL_ENABLED=true
        ULB->>OSDL_API: gRPC ScheduleService.StartWorkflow<br/>(lab_uuid, workflow_uuid, user_id)
        OSDL_API->>OSDL_API: 认证拦截器验证 Bearer token
        OSDL_API->>Redis: LPush lab_task_queue_{lab_uuid}<br/>{action: start_job, task_uuid, workflow}
        Redis-->>OSDL_API: OK
        OSDL_API-->>ULB: {task_uuid}
    else OSDL_ENABLED=false
        ULB->>Redis: LPush lab_task_queue_{lab_uuid}<br/>{action: start_job, task_uuid, workflow}
        Redis-->>ULB: OK
    end

    ULB-->>User: {task_uuid}

    Note over Redis,OSDL_SCH: 2. Schedule 服务获取任务

    OSDL_SCH->>Redis: BRPop lab_task_queue_{lab_uuid}
    Redis-->>OSDL_SCH: {action: start_job, task_uuid, workflow_dag}
    OSDL_SCH->>OSDL_SCH: 创建 DAG 引擎<br/>解析执行顺序<br/>找到首批可执行节点

    Note over OSDL_SCH,Edge: 3. 按拓扑顺序执行每个 DAG 节点

    loop 按拓扑顺序遍历每个节点
        OSDL_SCH->>Edge: session.Write({action: job_start,<br/>device_id, action_name, params})
        Edge->>Edge: 执行物理动作<br/>（泵送、加热、测量...）
        Edge-->>OSDL_SCH: {action: report_action_state,<br/>device_id, status: running}
        OSDL_SCH->>OSDL_SCH: SetDeviceActionStatus()
        Edge-->>OSDL_SCH: {action: report_action_state,<br/>device_id, status: completed}
        OSDL_SCH->>OSDL_SCH: 检查 DAG 完成状态 → 执行下一批节点
    end

    Edge-->>OSDL_SCH: {action: job_status, status: finished}
    OSDL_SCH->>Redis: Publish osdl:job:status:{task_uuid}<br/>{status: completed}

    Note over User,OSDL_API: 4. 客户端流式获取状态

    User->>OSDL_API: gRPC ScheduleService.StreamJobStatus(task_uuid)
    OSDL_API->>Redis: Subscribe osdl:job:status:{task_uuid}
    Redis-->>OSDL_API: 状态事件 (running → completed)
    OSDL_API-->>User: stream JobStatusEvent
```

### 5.2 单步 Action 执行（MCP）

```mermaid
sequenceDiagram
    participant App as 浏览器 / MCP 客户端
    participant ULB as uni-lab-backend :80
    participant OSDL_API as OSDL API :9090
    participant Redis as Redis
    participant OSDL_SCH as OSDL Schedule :8081
    participant Edge as Edge 设备

    App->>ULB: POST /api/v1/lab/mcp/run/action<br/>{lab_uuid, device_id, action, params}
    ULB->>ULB: 验证设备能力

    alt OSDL_ENABLED=true
        ULB->>OSDL_API: gRPC ScheduleService.StartAction<br/>(lab_uuid, device_id, action, param)
        OSDL_API->>Redis: LPush lab_control_queue_{lab_uuid}<br/>{action: start_action, task_uuid}
        OSDL_API-->>ULB: {task_uuid}
    else OSDL_ENABLED=false
        ULB->>Redis: LPush lab_control_queue_{lab_uuid}
        Redis-->>ULB: OK
    end

    ULB-->>App: {task_uuid}

    OSDL_SCH->>Redis: BRPop lab_control_queue_{lab_uuid}
    Redis-->>OSDL_SCH: {action: start_action, device_id, action_name}
    OSDL_SCH->>Edge: session.Write({action: job_start,<br/>device_id, action_name, params})
    Edge->>Edge: 执行动作
    Edge-->>OSDL_SCH: {action: job_status, status: finished}
    OSDL_SCH->>Redis: Publish osdl:job:status:{task_uuid}

    App->>ULB: GET /api/v1/lab/mcp/task/{task_uuid}
    ULB-->>App: {status: completed, result}
```

### 5.3 Edge 设备连接生命周期

```mermaid
sequenceDiagram
    participant Edge as Edge 设备
    participant WS as Schedule 服务 :8081<br/>(WebSocket 中心)
    participant Redis as Redis
    participant Engine as 执行引擎

    Note over Edge,WS: 1. 建立连接

    Edge->>WS: WebSocket 升级<br/>Header: Lab base64(AK:SK)
    WS->>WS: AuthLab 中间件<br/>解码 AK:SK → 查找实验室
    WS->>Redis: SetNX lab_heart_key_{lab_uuid}
    Redis-->>WS: OK（无其他 Edge 连接）
    WS->>WS: 创建 EdgeImpl 实例<br/>注册到 Hub map[lab_id]

    WS-->>Edge: 连接已建立

    Note over Edge,WS: 2. Edge 初始化

    Edge->>WS: {action: host_node_ready}
    WS->>WS: onEdgeReady()
    WS->>WS: 启动协程：<br/>• 任务队列消费者 (BRPop)<br/>• 控制队列消费者 (BRPop)<br/>• 心跳定时器

    Note over WS,Redis: 3. 运行时 — 并行循环

    par 任务队列消费者
        loop 持续运行（直到断开）
            WS->>Redis: BRPop lab_task_queue_{lab_uuid} (10s 超时)
            Redis-->>WS: 任务消息（或超时 → 重试）
            WS->>Engine: onJobMessage() → 创建 DAG/Notebook 引擎
        end
    and 控制队列消费者
        loop 持续运行
            WS->>Redis: BRPop lab_control_queue_{lab_uuid} (10s 超时)
            Redis-->>WS: 控制消息（action/stop/material）
            WS->>Engine: onControlMessage() → 执行动作
        end
    and 心跳保活
        loop 每个 LabHeartTime (10s)
            WS->>Redis: SetEx lab_heart_key_{lab_uuid} TTL=1000s
        end
    and 设备状态
        loop 设备状态变化时
            Edge->>WS: {action: device_status, device_id, property, value}
            WS->>WS: 更新 MaterialNode.Data 到数据库
            WS->>Redis: Publish osdl:device:status:{lab_uuid}
        end
    end

    Note over Edge,WS: 4. 优雅断开

    Edge->>WS: {action: normal_exit}
    WS->>WS: EdgeImpl.Close()
    WS->>Redis: Del lab_heart_key_{lab_uuid}
    WS->>WS: 取消所有协程<br/>从 Hub map 移除
```

### 5.4 物料图同步（定义 → 运行时）

```mermaid
sequenceDiagram
    participant User as 浏览器
    participant ULB as uni-lab-backend :80
    participant OSDL_API as OSDL API :8080
    participant DB as PostgreSQL
    participant OSDL_SCH as Schedule :8081
    participant Edge as Edge 设备
    participant SSE as SSE 订阅者

    Note over User,ULB: 阶段 1：用户在 UI 定义物料图

    User->>ULB: POST /api/v1/lab/material<br/>{lab_uuid, nodes, edges}
    ULB->>DB: INSERT material_nodes, material_edges
    DB-->>ULB: 创建成功
    ULB-->>User: 物料图已保存

    Note over Edge,OSDL_API: 阶段 2：Edge 上报物理拓扑

    Edge->>OSDL_API: POST /api/v1/edge/material/create<br/>Header: Lab AK/SK<br/>{lab_uuid, devices: [...]}
    OSDL_API->>DB: INSERT material_nodes（物理设备）
    DB-->>OSDL_API: 创建成功
    OSDL_API-->>Edge: {items: [{uuid, name}, ...]}

    Edge->>OSDL_API: POST /api/v1/edge/material/upsert<br/>{lab_uuid, nodes: [{device_id, data, schema}]}
    OSDL_API->>DB: UPSERT material_nodes（设备数据 + schema）
    DB-->>OSDL_API: 更新成功
    OSDL_API-->>Edge: OK

    Note over Edge,SSE: 阶段 3：运行时设备状态更新

    Edge->>OSDL_SCH: WebSocket: {action: device_status,<br/>device_id: pump-1, temperature: 25.3}
    OSDL_SCH->>DB: UPDATE material_nodes<br/>SET data['temperature'] = 25.3<br/>WHERE device_id = 'pump-1'
    OSDL_SCH->>OSDL_SCH: Redis Publish osdl:device:status:{lab_uuid}
    OSDL_SCH-->>SSE: SSE 事件: material_modify<br/>{node_uuid, key: temperature, value: 25.3}

    Note over User,SSE: 阶段 4：UI 接收实时更新

    User->>OSDL_API: GET /api/v1/lab/notify/sse<br/>EventSource 连接
    OSDL_API-->>User: SSE: material_modify 事件<br/>→ UI 实时更新设备面板
```

### 5.5 OAuth2 登录流程

```mermaid
sequenceDiagram
    participant User as 浏览器
    participant OSDL as OSDL API :8080
    participant Redis as Redis
    participant CAS as Casdoor :8000

    User->>OSDL: GET /api/auth/login?frontend_callback_url=https://studio.example.com/callback
    OSDL->>OSDL: 生成随机 state
    OSDL->>Redis: SET state → frontend_callback_url (TTL 5分钟)
    OSDL-->>User: 302 → Casdoor 授权页<br/>?client_id=...&state=...&redirect_uri=/api/auth/callback/casdoor

    User->>CAS: 用户名/密码登录
    CAS-->>User: 302 → /api/auth/callback/casdoor?code=xxx&state=yyy

    User->>OSDL: GET /api/auth/callback/casdoor?code=xxx&state=yyy
    OSDL->>Redis: GET state → frontend_callback_url
    Redis-->>OSDL: https://studio.example.com/callback
    OSDL->>CAS: POST /api/login/oauth/access_token<br/>{code, client_id, client_secret}
    CAS-->>OSDL: {access_token, refresh_token, expires_in}
    OSDL->>CAS: GET /api/get-account<br/>Authorization: Bearer {access_token}
    CAS-->>OSDL: {id, name, email, avatar, ...}
    OSDL-->>User: 302 → https://studio.example.com/callback<br/>?access_token=...&refresh_token=...

    Note over User,OSDL: Token 刷新（access_token 过期时）

    User->>OSDL: POST /api/auth/refresh<br/>{refresh_token}
    OSDL->>CAS: POST /api/login/oauth/access_token<br/>{grant_type: refresh_token}
    CAS-->>OSDL: {new_access_token, new_refresh_token}
    OSDL-->>User: {access_token, refresh_token, expires_in}
```

---

## 6. 集成点 — Dispatcher 接口

uni-lab-backend 和 OSDL 之间的**唯一集成点**是 `Dispatcher` 接口：

```go
type Dispatcher interface {
    StartWorkflow(ctx, labUUID, workflowUUID, userID) (taskUUID, error)
    StartNotebook(ctx, labUUID, notebookUUID, userID) (taskUUID, error)
    StartAction(ctx, labUUID, deviceID, action, actionType, param) (taskUUID, error)
    StopJob(ctx, taskUUID, userID) error
}
```

```mermaid
graph LR
    subgraph ULB["uni-lab-backend"]
        WF_RUN["PUT /run/workflow"]
        NB_RUN["PUT /notebook/run"]
        MCP_RUN["POST /mcp/run/action"]
        STOP["StopJob"]
    end

    DISP{OSDL_ENABLED?}

    subgraph Redis_Direct["RedisDispatcher"]
        LP1["LPush lab_task_queue"]
        LP2["LPush lab_control_queue"]
        PUB["Publish osdl:job:stop:*"]
    end

    subgraph OSDL_GRPC["OSDLDispatcher"]
        G1["gRPC StartWorkflow"]
        G2["gRPC StartNotebook"]
        G3["gRPC StartAction"]
        G4["gRPC StopJob"]
    end

    WF_RUN & NB_RUN & MCP_RUN & STOP --> DISP
    DISP -->|false| Redis_Direct
    DISP -->|true| OSDL_GRPC
    OSDL_GRPC -->|内部| LP1 & LP2 & PUB
```

**回滚**：设置 `OSDL_ENABLED=false` → 即时回退到直接 Redis，无需数据迁移。

---

## 7. Redis 键参考

| 键模式 | 类型 | 所有者 | TTL | 用途 |
|--------|------|--------|-----|------|
| `lab_task_queue_{lab_uuid}` | List | OSDL / ULB | — | 工作流 + 实验本任务分发 |
| `lab_control_queue_{lab_uuid}` | List | OSDL / ULB | — | Action + 停止 + 物料控制 |
| `lab_heart_key_{lab_uuid}` | String | OSDL Schedule | 1000s | Edge 设备存活检测（心跳） |
| `osdl:job:status:{task_uuid}` | Pub/Sub | OSDL Schedule | — | 任务状态流式频道 |
| `osdl:job:stop:{task_uuid}` | Pub/Sub | OSDL API | — | 停止命令广播 |
| `osdl:device:status:{lab_uuid}` | Pub/Sub | OSDL Schedule | — | 设备状态流式频道 |
| OAuth state key | String | OSDL API | 5min | OAuth2 state → callback_url 映射 |

---

## 8. 认证矩阵

| 认证类型 | Header 格式 | 验证方式 | 使用场景 |
|----------|------------|----------|----------|
| **Bearer** | `Authorization: Bearer <token>` | Casdoor UserInfo / Bohrium RSA JWT | Web 用户（浏览器、SDK） |
| **Lab** | `Authorization: Lab base64(AK:SK)` | 数据库查询 | Edge 设备 |
| **Api** | `Authorization: Bearer <jwt>` | Bohrium RSA 公钥 | API 集成 |
| **gRPC metadata** | `authorization: Bearer <token>` | 认证拦截器 → ValidateToken | 上游 gRPC 客户端 |

通过 `OAUTH_SOURCE` 环境变量切换：
- `casdoor`（默认）：Casdoor OAuth2 授权码 + UserInfo 端点
- `bohr`：Bohrium JWT + RSA 公钥验证 (JWKS)

---

## 9. WebSocket 消息协议

### Edge → OSDL（Schedule 服务）

| 动作 | 载荷 | 触发条件 |
|------|------|----------|
| `host_node_ready` | `{}` | Edge 初始化完成 |
| `device_status` | `{device_id, property, value}` | 设备状态变化 |
| `report_action_state` | `{device_id, action, status}` | 动作执行进度 |
| `job_status` | `{job_id, status, data}` | 任务完成上报 |
| `ping` | `{}` | 心跳请求 |
| `normal_exit` | `{}` | 优雅断开 |

### OSDL → Edge（Schedule 服务）

| 动作 | 载荷 | 触发条件 |
|------|------|----------|
| `job_start` | `{device_id, action, params}` | 任务下发到设备 |
| `query_action_state` | `{device_id, action}` | 查询设备能力 |
| `cancel_task` | `{task_uuid}` | 停止运行中的任务 |
| `task_finished` | `{task_uuid}` | 任务完成通知 |
| `add_material` | `{node data}` | 物料拓扑更新 |
| `update_material` | `{node data}` | 物料数据更新 |
| `remove_material` | `{node_uuid}` | 物料节点移除 |
| `pong` | `{}` | 心跳响应 |

---

## 10. 部署拓扑

### 开发环境（Docker Compose）

```
┌─ docker/docker-compose.infra.yaml ─────────────────────┐
│  network-service (alpine) ← 共享网络命名空间             │
│  ├── postgresql :5432   (db-data 卷)                    │
│  ├── redis :6379        (redis-data 卷)                 │
│  └── casdoor :8000      (可选)                          │
└─────────────────────────────────────────────────────────┘

┌─ docker/docker-compose.base.yaml + dev.yaml ────────────┐
│  network-service (alpine) ← 开发端口 :8080 :8081 :9090  │
│  ├── api (golang:1.24-alpine)                           │
│  │   └── make dev (air 热重载 .air.web.toml)            │
│  └── schedule (golang:1.24-alpine)                      │
│      └── make dev-schedule (air .air.schedule.toml)     │
│                                                         │
│  卷: 源码挂载, go-mod-cache, go-bin-cache               │
│  环境: host.docker.internal 连接 PG / Redis / Casdoor    │
└─────────────────────────────────────────────────────────┘
```

### 生产环境

```
┌─ docker-compose.yml ────────────────────────────────────┐
│  postgres :5432    (pgdata 卷, 健康检查)                 │
│  redis :6379       (redisdata 卷, 健康检查)              │
│  osdl-migrate      (一次性, depends_on: pg healthy)      │
│  osdl-api :8080 :9090  (depends_on: pg, redis, migrate) │
│  osdl-schedule :8081   (depends_on: pg, redis, migrate) │
└─────────────────────────────────────────────────────────┘
```

---

## 11. 迁移策略

| 阶段 | 描述 | 开关 | 风险 |
|------|------|------|------|
| **0. 并行部署** | 在 uni-lab-backend 旁部署 OSDL，`OSDL_ENABLED=false` | `false` | 无 — 行为不变 |
| **1. 灰度** | 为 1 个测试实验室启用，监控 gRPC 延迟 + 任务完成率 | `true`（按实验室） | 低 — 仅影响单个实验室 |
| **2. 逐步推广** | 为所有实验室启用，双写模式对比 | `true` | 中 — 监控 Redis 队列深度 |
| **3. 全量迁移** | 所有 Edge 连接通过 OSDL Schedule 服务 | `true` | — |
| **4. 清理** | 移除 RedisDispatcher 代码，移除 uni-lab-backend 中的 schedule 服务 | N/A | — |

**任意阶段回滚**：设置 `OSDL_ENABLED=false`，重启 uni-lab-backend。即时生效，无需数据迁移。

---

## 12. 功能划分总览

```
┌─────────────────────────────────────────────────────────────────────┐
│                      uni-lab-backend (75%)                          │
│                                                                     │
│  用户 & 组织 ─── 实验室 ─── RBAC ─── OPA 策略                      │
│  工作流模板 ─── 节点模板 ─── 版本管理                                │
│  实验本模板 ─── 样本 ─── Schema                                     │
│  审批工作流 ─── 通知 CRUD                                           │
│  试剂 CRUD ─── PubChem ─── CAS 查询                                │
│  文件存储 ─── OSS 预签名 URL                                        │
│  动态配置 (Nacos) ─── 功能开关                                      │
│  任务历史 ─── 任务下载                                              │
│                                                                     │
│  Dispatcher ─────────────────────────────┐                         │
│    ├── RedisDispatcher (OSDL_ENABLED=false)                        │
│    └── OSDLDispatcher (OSDL_ENABLED=true) ──→ gRPC                │
└─────────────────────────────────────────────┼───────────────────────┘
                                              │
                                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                            OSDL (25%)                               │
│                                                                     │
│  ┌─ API 服务 ───────────────────────────────────────────────┐      │
│  │  OAuth2 (Casdoor / Bohrium) ─── Token 验证                │      │
│  │  物料 CRUD (Web + Edge) ─── 物料 WebSocket                │      │
│  │  gRPC 服务 (4个) ─── 认证拦截器                            │      │
│  │  SSE 通知 ─── 健康检查 ─── Swagger UI                     │      │
│  └───────────────────────────────────────────────────────────┘      │
│                                                                     │
│  ┌─ Schedule 服务 ──────────────────────────────────────────┐      │
│  │  WebSocket 中心 (Melody, 200 连接)                        │      │
│  │  任务队列消费者 (BRPop) ─── 控制队列消费者                 │      │
│  │  DAG 引擎 ─── Notebook 引擎 ─── Action 引擎              │      │
│  │  心跳监控 ─── 设备状态广播                                 │      │
│  └───────────────────────────────────────────────────────────┘      │
│                                                                     │
│  Edge 设备 ←──── WebSocket :8081 ────→ 物理实验室硬件               │
└─────────────────────────────────────────────────────────────────────┘
```
