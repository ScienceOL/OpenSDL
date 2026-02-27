# OSDL System Architecture — Complete Functional Division & Data Flow

This document provides a comprehensive view of the **OSDL + uni-lab-backend + Edge** three-tier architecture, covering the complete functional boundary, data ownership, integration points, and runtime data flow.

> **Language / 语言**: English | [中文版](./ARCHITECTURE_CN.md)

---

## 1. System Overview

The full-stack scientific laboratory platform consists of three independent deployments:

| Component | Role | Ports | Technology |
|-----------|------|-------|------------|
| **uni-lab-backend** | Business logic, user management, templates, approval | HTTP :80 | Go + Gin + GORM + PostgreSQL |
| **OSDL API Server** | Edge communication gateway, gRPC API, OAuth2 | HTTP :8080 + gRPC :9090 | Go + Gin + gRPC + Redis |
| **OSDL Schedule Server** | WebSocket hub, task execution engine | WS :8081 | Go + Melody + Redis |
| **Edge Devices** | Physical lab instruments (Uni-Lab-Edge / unilabos) | — | Python / C++ |

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                        Browser / Client / Studio                            │
└──────┬──────────────────────────────┬───────────────────────────────────────┘
       │ HTTP :80                     │ HTTP :8080 / gRPC :9090
       ▼                              ▼
┌──────────────────┐           ┌──────────────────────────┐
│ uni-lab-backend   │──gRPC───→│    OSDL API Server       │
│                   │          │                          │
│ Users / Workflows │          │  Material CRUD           │
│ Notebooks / RBAC  │          │  OAuth2 (Casdoor/Bohr)  │
│ Approval / Storage│          │  gRPC Services (×4)      │
│ Reagent / OPA     │          │  SSE Notifications       │
│ Nacos / Templates │          │  Swagger UI              │
└──────────────────┘           └────────────┬─────────────┘
                                            │ Redis Queues + Pub/Sub
                                            ▼
                               ┌──────────────────────────┐
                               │  OSDL Schedule Server     │
                               │                          │
                               │  WebSocket Hub (Melody)  │
                               │  Task Queue Consumer     │
                               │  Control Queue Consumer  │
                               │  Engine: DAG / Notebook  │
                               │          / Action        │
                               └────────────┬─────────────┘
                                            │ WebSocket :8081
                                            ▼
                               ┌──────────────────────────┐
                               │  Edge Devices             │
                               │  (Uni-Lab-Edge / unilabos)│
                               └──────────────────────────┘
```

---

## 2. Mermaid Architecture Diagram

```mermaid
graph TB
    subgraph Client["Browser / Studio"]
        WEB[Web UI]
        SDK[Client SDK]
    end

    subgraph ULB["uni-lab-backend :80"]
        direction TB
        ULB_API[REST API]
        ULB_AUTH[User Auth]
        subgraph ULB_BIZ["Business Logic"]
            ENV[Lab / User / Org]
            WF[Workflow Templates]
            NB[Notebook Templates]
            RBAC[RBAC / OPA Policy]
            APPR[Approval Workflows]
            REAG[Reagent Management]
            STOR[File Storage / OSS]
        end
        DISP{Dispatcher<br/>OSDL_ENABLED?}
    end

    subgraph OSDL["OSDL Platform"]
        subgraph OSDL_API["API Server :8080 / :9090"]
            HTTP_API[HTTP REST + Swagger]
            GRPC_SVC["gRPC Services<br/>Schedule / Material<br/>Edge / Auth"]
            OAUTH["OAuth2<br/>(Casdoor / Bohrium)"]
            MAT_CRUD[Material CRUD]
            SSE[SSE Notifications]
            INTERCEPTOR[Auth Interceptor]
        end
        subgraph OSDL_SCH["Schedule Server :8081"]
            WS_HUB["WebSocket Hub<br/>(Melody)"]
            TQ["Task Queue<br/>(Redis BRPop)"]
            CQ["Control Queue<br/>(Redis BRPop)"]
            ENGINE["Execution Engine<br/>DAG / Notebook / Action"]
            HEARTBEAT["Heartbeat Monitor"]
        end
    end

    subgraph Infra["Infrastructure"]
        PG[(PostgreSQL)]
        RD[(Redis)]
        CAS["Casdoor :8000"]
    end

    subgraph Edge["Edge Devices"]
        E1["Device 1<br/>(pump, reactor...)"]
        E2["Device 2"]
        EN["Device N"]
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

## 3. Complete Functional Division

### 3.1 uni-lab-backend (75% — Business Logic Layer)

| Domain | Module | Key Operations | Data Store |
|--------|--------|----------------|------------|
| **User / Org** | `core/environment/` | User profile, lab CRUD, member invite/remove, lab pin | PostgreSQL |
| **Authorization** | `core/inner/` + OPA | RBAC roles, role permissions, user-role binding, custom policies | PostgreSQL + OPA |
| **Workflow Templates** | `core/workflow/` | Template CRUD, fork, import/export, versioning, tagging | PostgreSQL |
| **Workflow Node Templates** | `core/workflow/` | Node template CRUD, schema definition, device capability query | PostgreSQL |
| **Notebook Templates** | `core/notebook/` | Notebook CRUD, sample tracking, schema definition | PostgreSQL |
| **Material Definition** | `core/material/` (UI) | Material graph creation in web UI, template management | PostgreSQL |
| **Reagent** | `core/reagent/` | Reagent CRUD, CAS lookup (PubChem API) | PostgreSQL |
| **Approval** | `core/sse/` | Workflow submission, approval chain, approve/reject | PostgreSQL |
| **File Storage** | `core/storage/` | Pre-signed URL generation, OSS integration | OSS (S3) |
| **Notifications** | `core/notify/` + `core/sse/` | SSE stream, notification CRUD, cross-pod broadcast | Redis Pub/Sub |
| **Dynamic Config** | Nacos | Hot-reload configuration, feature flags | Nacos |
| **Task Dispatch** | `core/schedule/` | Workflow/Notebook/Action dispatch via `Dispatcher` interface | — |
| **Task History** | `web/views/workflow/` | Task list, task download, status query | PostgreSQL |

### 3.2 OSDL (25% — Edge Communication & Scheduling Layer)

| Domain | Module | Key Operations | Data Store |
|--------|--------|----------------|------------|
| **Edge Connectivity** | `core/schedule/control/` | WebSocket session lifecycle (Melody), 200-conn pool | In-memory |
| **Edge Auth** | `middleware/auth/` | Lab AK/SK header validation (`Lab base64(AK:SK)`) | PostgreSQL |
| **Heartbeat** | `core/schedule/lab/edge/` | Periodic `SetEx` to `lab_heart_key_{lab_uuid}` (TTL 1000s) | Redis |
| **Task Execution** | `core/schedule/engine/` | DAG workflow, Notebook, single Action execution | Redis |
| **Queue Consumption** | `core/schedule/lab/edge/` | BRPop from `lab_task_queue_*` and `lab_control_queue_*` | Redis |
| **Material Runtime** | `core/material/` | Edge-reported material create/upsert, device status sync | PostgreSQL |
| **Device Status** | `core/schedule/lab/edge/` | Device property updates broadcast via Redis Pub/Sub | Redis Pub/Sub |
| **OAuth2** | `core/login/casdoor/` + `repo/bohr/` | Casdoor OAuth2 + Bohrium JWT, switchable via `OAUTH_SOURCE` | Redis (state) |
| **gRPC API** | `pkg/grpc/services/` | 4 services, 14 RPCs for upstream integration | Redis + PostgreSQL |
| **HTTP API** | `pkg/web/` | Material CRUD, health checks, SSE, Swagger UI | PostgreSQL |

### 3.3 Edge Devices (Physical Layer)

| Domain | Operations |
|--------|-----------|
| **Device Control** | Execute actions (pump, heat, stir, measure...) |
| **Status Reporting** | Push device_status messages via WebSocket |
| **Material Topology** | Report physical device graph (POST to OSDL Edge API) |
| **Task Execution** | Receive action commands, report completion via job_status |
| **Heartbeat** | Periodic ping/pong to maintain connection liveness |

---

## 4. Data Ownership

```mermaid
graph LR
    subgraph PostgreSQL["PostgreSQL (Persistent)"]
        direction TB
        ULB_DATA["uni-lab-backend owns:<br/>• users, organizations<br/>• workflows, notebooks<br/>• approvals, reagents<br/>• roles, permissions<br/>• storage tokens, tags"]
        OSDL_DATA["OSDL owns:<br/>• material_nodes<br/>• material_edges<br/>• lab environments<br/>• sandbox configs"]
    end

    subgraph Redis["Redis (Runtime)"]
        direction TB
        QUEUES["Task Queues:<br/>• lab_task_queue_{uuid}<br/>• lab_control_queue_{uuid}"]
        PUBSUB["Pub/Sub Channels:<br/>• osdl:job:status:{uuid}<br/>• osdl:job:stop:{uuid}<br/>• osdl:device:status:{uuid}"]
        HEARTBEAT_KEY["Heartbeat Keys:<br/>• lab_heart_key_{uuid}<br/>(TTL-based liveness)"]
        OAUTH_STATE["OAuth State:<br/>• state → callback_url<br/>(5min TTL)"]
    end

    subgraph Memory["In-Memory (Volatile)"]
        WS_SESSIONS["WebSocket Sessions:<br/>• EdgeImpl per lab<br/>• Melody session pool"]
        TASK_INSTANCES["Task Instances:<br/>• Running DAG engines<br/>• Running Notebook engines<br/>• Running Action tasks"]
    end
```

---

## 5. Sequence Diagrams

### 5.1 Workflow Execution (Full Path)

```mermaid
sequenceDiagram
    participant User as Browser
    participant ULB as uni-lab-backend :80
    participant OSDL_API as OSDL API :9090
    participant Redis as Redis
    participant OSDL_SCH as OSDL Schedule :8081
    participant Edge as Edge Device

    Note over User,ULB: 1. User triggers workflow execution

    User->>ULB: PUT /api/v1/lab/run/workflow<br/>{workflow_uuid, lab_uuid}
    ULB->>ULB: Validate workflow template<br/>Check approval status<br/>Resolve device bindings

    alt OSDL_ENABLED=true
        ULB->>OSDL_API: gRPC ScheduleService.StartWorkflow<br/>(lab_uuid, workflow_uuid, user_id)
        OSDL_API->>OSDL_API: Auth Interceptor validates Bearer token
        OSDL_API->>Redis: LPush lab_task_queue_{lab_uuid}<br/>{action: start_job, task_uuid, workflow}
        Redis-->>OSDL_API: OK
        OSDL_API-->>ULB: {task_uuid}
    else OSDL_ENABLED=false
        ULB->>Redis: LPush lab_task_queue_{lab_uuid}<br/>{action: start_job, task_uuid, workflow}
        Redis-->>ULB: OK
    end

    ULB-->>User: {task_uuid}

    Note over Redis,OSDL_SCH: 2. Schedule server picks up task

    OSDL_SCH->>Redis: BRPop lab_task_queue_{lab_uuid}
    Redis-->>OSDL_SCH: {action: start_job, task_uuid, workflow_dag}
    OSDL_SCH->>OSDL_SCH: Create DAG engine<br/>Resolve execution order<br/>Find first executable nodes

    Note over OSDL_SCH,Edge: 3. Execute each DAG node

    loop For each node in topological order
        OSDL_SCH->>Edge: session.Write({action: job_start,<br/>device_id, action_name, params})
        Edge->>Edge: Execute physical action<br/>(pump, heat, measure...)
        Edge-->>OSDL_SCH: {action: report_action_state,<br/>device_id, status: running}
        OSDL_SCH->>OSDL_SCH: SetDeviceActionStatus()
        Edge-->>OSDL_SCH: {action: report_action_state,<br/>device_id, status: completed}
        OSDL_SCH->>OSDL_SCH: Check DAG completion → next nodes
    end

    Edge-->>OSDL_SCH: {action: job_status, status: finished}
    OSDL_SCH->>Redis: Publish osdl:job:status:{task_uuid}<br/>{status: completed}

    Note over User,OSDL_API: 4. Client streams status updates

    User->>OSDL_API: gRPC ScheduleService.StreamJobStatus(task_uuid)
    OSDL_API->>Redis: Subscribe osdl:job:status:{task_uuid}
    Redis-->>OSDL_API: Status events (running → completed)
    OSDL_API-->>User: stream JobStatusEvent
```

### 5.2 Single Action Execution (MCP)

```mermaid
sequenceDiagram
    participant App as Browser / MCP Client
    participant ULB as uni-lab-backend :80
    participant OSDL_API as OSDL API :9090
    participant Redis as Redis
    participant OSDL_SCH as OSDL Schedule :8081
    participant Edge as Edge Device

    App->>ULB: POST /api/v1/lab/mcp/run/action<br/>{lab_uuid, device_id, action, params}
    ULB->>ULB: Validate device capability

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
    Edge->>Edge: Execute action
    Edge-->>OSDL_SCH: {action: job_status, status: finished}
    OSDL_SCH->>Redis: Publish osdl:job:status:{task_uuid}

    App->>ULB: GET /api/v1/lab/mcp/task/{task_uuid}
    ULB-->>App: {status: completed, result}
```

### 5.3 Edge Device Connection Lifecycle

```mermaid
sequenceDiagram
    participant Edge as Edge Device
    participant WS as Schedule Server :8081<br/>(WebSocket Hub)
    participant Redis as Redis
    participant Engine as Execution Engine

    Note over Edge,WS: 1. Connection establishment

    Edge->>WS: WebSocket upgrade<br/>Header: Lab base64(AK:SK)
    WS->>WS: AuthLab middleware<br/>Decode AK:SK → lookup lab
    WS->>Redis: SetNX lab_heart_key_{lab_uuid}
    Redis-->>WS: OK (no other Edge connected)
    WS->>WS: Create EdgeImpl instance<br/>Register in Hub map[lab_id]

    WS-->>Edge: Connection established

    Note over Edge,WS: 2. Edge initialization

    Edge->>WS: {action: host_node_ready}
    WS->>WS: onEdgeReady()
    WS->>WS: Start goroutines:<br/>• Task queue consumer (BRPop)<br/>• Control queue consumer (BRPop)<br/>• Heartbeat ticker

    Note over WS,Redis: 3. Runtime — parallel loops

    par Task queue consumer
        loop Forever (until disconnect)
            WS->>Redis: BRPop lab_task_queue_{lab_uuid} (10s timeout)
            Redis-->>WS: Task message (or timeout → retry)
            WS->>Engine: onJobMessage() → create DAG/Notebook engine
        end
    and Control queue consumer
        loop Forever
            WS->>Redis: BRPop lab_control_queue_{lab_uuid} (10s timeout)
            Redis-->>WS: Control message (action/stop/material)
            WS->>Engine: onControlMessage() → execute action
        end
    and Heartbeat
        loop Every LabHeartTime (10s)
            WS->>Redis: SetEx lab_heart_key_{lab_uuid} TTL=1000s
        end
    and Device status
        loop On device state change
            Edge->>WS: {action: device_status, device_id, property, value}
            WS->>WS: Update MaterialNode.Data in DB
            WS->>Redis: Publish osdl:device:status:{lab_uuid}
        end
    end

    Note over Edge,WS: 4. Graceful disconnect

    Edge->>WS: {action: normal_exit}
    WS->>WS: EdgeImpl.Close()
    WS->>Redis: Del lab_heart_key_{lab_uuid}
    WS->>WS: Cancel all goroutines<br/>Remove from Hub map
```

### 5.4 Material Graph Sync (Definition → Runtime)

```mermaid
sequenceDiagram
    participant User as Browser
    participant ULB as uni-lab-backend :80
    participant OSDL_API as OSDL API :8080
    participant DB as PostgreSQL
    participant OSDL_SCH as Schedule :8081
    participant Edge as Edge Device
    participant SSE as SSE Subscribers

    Note over User,ULB: Phase 1: User defines material graph in UI

    User->>ULB: POST /api/v1/lab/material<br/>{lab_uuid, nodes, edges}
    ULB->>DB: INSERT material_nodes, material_edges
    DB-->>ULB: Created
    ULB-->>User: Material graph saved

    Note over Edge,OSDL_API: Phase 2: Edge reports physical topology

    Edge->>OSDL_API: POST /api/v1/edge/material/create<br/>Header: Lab AK/SK<br/>{lab_uuid, devices: [...]}
    OSDL_API->>DB: INSERT material_nodes (physical devices)
    DB-->>OSDL_API: Created
    OSDL_API-->>Edge: {items: [{uuid, name}, ...]}

    Edge->>OSDL_API: POST /api/v1/edge/material/upsert<br/>{lab_uuid, nodes: [{device_id, data, schema}]}
    OSDL_API->>DB: UPSERT material_nodes (device data + schema)
    DB-->>OSDL_API: Updated
    OSDL_API-->>Edge: OK

    Note over Edge,SSE: Phase 3: Runtime device status updates

    Edge->>OSDL_SCH: WebSocket: {action: device_status,<br/>device_id: pump-1, temperature: 25.3}
    OSDL_SCH->>DB: UPDATE material_nodes<br/>SET data['temperature'] = 25.3<br/>WHERE device_id = 'pump-1'
    OSDL_SCH->>OSDL_SCH: Redis Publish osdl:device:status:{lab_uuid}
    OSDL_SCH-->>SSE: SSE event: material_modify<br/>{node_uuid, key: temperature, value: 25.3}

    Note over User,SSE: Phase 4: UI receives real-time updates

    User->>OSDL_API: GET /api/v1/lab/notify/sse<br/>EventSource connection
    OSDL_API-->>User: SSE: material_modify event<br/>→ UI updates device panel in real-time
```

### 5.5 OAuth2 Login Flow

```mermaid
sequenceDiagram
    participant User as Browser
    participant OSDL as OSDL API :8080
    participant Redis as Redis
    participant CAS as Casdoor :8000

    User->>OSDL: GET /api/auth/login?frontend_callback_url=https://studio.example.com/callback
    OSDL->>OSDL: Generate random state
    OSDL->>Redis: SET state → frontend_callback_url (TTL 5min)
    OSDL-->>User: 302 → Casdoor authorize URL<br/>?client_id=...&state=...&redirect_uri=/api/auth/callback/casdoor

    User->>CAS: Login with username/password
    CAS-->>User: 302 → /api/auth/callback/casdoor?code=xxx&state=yyy

    User->>OSDL: GET /api/auth/callback/casdoor?code=xxx&state=yyy
    OSDL->>Redis: GET state → frontend_callback_url
    Redis-->>OSDL: https://studio.example.com/callback
    OSDL->>CAS: POST /api/login/oauth/access_token<br/>{code, client_id, client_secret}
    CAS-->>OSDL: {access_token, refresh_token, expires_in}
    OSDL->>CAS: GET /api/get-account<br/>Authorization: Bearer {access_token}
    CAS-->>OSDL: {id, name, email, avatar, ...}
    OSDL-->>User: 302 → https://studio.example.com/callback<br/>?access_token=...&refresh_token=...

    Note over User,OSDL: Token refresh (when access_token expires)

    User->>OSDL: POST /api/auth/refresh<br/>{refresh_token}
    OSDL->>CAS: POST /api/login/oauth/access_token<br/>{grant_type: refresh_token}
    CAS-->>OSDL: {new_access_token, new_refresh_token}
    OSDL-->>User: {access_token, refresh_token, expires_in}
```

---

## 6. Integration Point — Dispatcher Interface

The **single integration point** between uni-lab-backend and OSDL is the `Dispatcher` interface:

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
    OSDL_GRPC -->|internally| LP1 & LP2 & PUB
```

**Rollback**: Set `OSDL_ENABLED=false` → instant fallback to direct Redis, no data migration needed.

---

## 7. Redis Key Reference

| Key Pattern | Type | Owner | TTL | Purpose |
|-------------|------|-------|-----|---------|
| `lab_task_queue_{lab_uuid}` | List | OSDL / ULB | — | Workflow + Notebook task dispatch |
| `lab_control_queue_{lab_uuid}` | List | OSDL / ULB | — | Action + Stop + Material control |
| `lab_heart_key_{lab_uuid}` | String | OSDL Schedule | 1000s | Edge device liveness (heartbeat) |
| `osdl:job:status:{task_uuid}` | Pub/Sub | OSDL Schedule | — | Job status streaming channel |
| `osdl:job:stop:{task_uuid}` | Pub/Sub | OSDL API | — | Stop command broadcast |
| `osdl:device:status:{lab_uuid}` | Pub/Sub | OSDL Schedule | — | Device status streaming channel |
| OAuth state key | String | OSDL API | 5min | OAuth2 state → callback_url mapping |

---

## 8. Authentication Matrix

| Auth Type | Header Format | Validator | Used By |
|-----------|---------------|-----------|---------|
| **Bearer** | `Authorization: Bearer <token>` | Casdoor UserInfo / Bohrium RSA JWT | Web users (browser, SDK) |
| **Lab** | `Authorization: Lab base64(AK:SK)` | Database lookup | Edge devices |
| **Api** | `Authorization: Bearer <jwt>` | Bohrium RSA public key | API integrations |
| **gRPC metadata** | `authorization: Bearer <token>` | Auth interceptor → ValidateToken | Upstream gRPC clients |

Switchable via `OAUTH_SOURCE` environment variable:
- `casdoor` (default): Casdoor OAuth2 Authorization Code + UserInfo endpoint
- `bohr`: Bohrium JWT with RSA public key verification (JWKS)

---

## 9. WebSocket Message Protocol

### Edge → OSDL (Schedule Server)

| Action | Payload | Trigger |
|--------|---------|---------|
| `host_node_ready` | `{}` | Edge initialization complete |
| `device_status` | `{device_id, property, value}` | Device state change |
| `report_action_state` | `{device_id, action, status}` | Action execution progress |
| `job_status` | `{job_id, status, data}` | Task completion report |
| `ping` | `{}` | Heartbeat request |
| `normal_exit` | `{}` | Graceful disconnect |

### OSDL → Edge (Schedule Server)

| Action | Payload | Trigger |
|--------|---------|---------|
| `job_start` | `{device_id, action, params}` | Task dispatch to device |
| `query_action_state` | `{device_id, action}` | Check device capability |
| `cancel_task` | `{task_uuid}` | Stop running task |
| `task_finished` | `{task_uuid}` | Task completion notification |
| `add_material` | `{node data}` | Material topology update |
| `update_material` | `{node data}` | Material data update |
| `remove_material` | `{node_uuid}` | Material node removal |
| `pong` | `{}` | Heartbeat response |

---

## 10. Deployment Topology

### Development (Docker Compose)

```
┌─ docker/docker-compose.infra.yaml ─────────────────────┐
│  network-service (alpine) ← shared network namespace    │
│  ├── postgresql :5432   (db-data volume)                │
│  ├── redis :6379        (redis-data volume)             │
│  └── casdoor :8000      (optional)                      │
└─────────────────────────────────────────────────────────┘

┌─ docker/docker-compose.base.yaml + dev.yaml ────────────┐
│  network-service (alpine) ← dev ports :8080 :8081 :9090 │
│  ├── api (golang:1.24-alpine)                           │
│  │   └── make dev (air hot-reload .air.web.toml)        │
│  └── schedule (golang:1.24-alpine)                      │
│      └── make dev-schedule (air .air.schedule.toml)     │
│                                                         │
│  Volumes: source mount, go-mod-cache, go-bin-cache      │
│  Env: host.docker.internal for PG / Redis / Casdoor     │
└─────────────────────────────────────────────────────────┘
```

### Production

```
┌─ docker-compose.yml ────────────────────────────────────┐
│  postgres :5432    (pgdata volume, healthcheck)         │
│  redis :6379       (redisdata volume, healthcheck)      │
│  osdl-migrate      (one-shot, depends_on: pg healthy)   │
│  osdl-api :8080 :9090  (depends_on: pg, redis, migrate) │
│  osdl-schedule :8081   (depends_on: pg, redis, migrate) │
└─────────────────────────────────────────────────────────┘
```

---

## 11. Migration Strategy

| Phase | Description | Flag | Risk |
|-------|-------------|------|------|
| **0. Side-by-side** | Deploy OSDL alongside uni-lab-backend, `OSDL_ENABLED=false` | `false` | None — no behavioral change |
| **1. Canary** | Enable for 1 test lab, monitor gRPC latency + task completion | `true` (per-lab) | Low — single lab affected |
| **2. Gradual rollout** | Enable for all labs, dual-write mode for comparison | `true` | Medium — monitor Redis queue depth |
| **3. Full migration** | All Edge connections through OSDL Schedule server | `true` | — |
| **4. Cleanup** | Remove RedisDispatcher code, schedule server from uni-lab-backend | N/A | — |

**Rollback at any phase**: Set `OSDL_ENABLED=false`, restart uni-lab-backend. Instant, no data migration.

---

## 12. Functional Division Summary

```
┌─────────────────────────────────────────────────────────────────────┐
│                        uni-lab-backend (75%)                        │
│                                                                     │
│  Users & Orgs ─── Labs ─── RBAC ─── OPA Policy                    │
│  Workflow Templates ─── Node Templates ─── Versioning              │
│  Notebook Templates ─── Samples ─── Schema                        │
│  Approval Workflows ─── Notification CRUD                          │
│  Reagent CRUD ─── PubChem ─── CAS Lookup                          │
│  File Storage ─── OSS Pre-signed URLs                              │
│  Dynamic Config (Nacos) ─── Feature Flags                          │
│  Task History ─── Task Download                                    │
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
│  ┌─ API Server ─────────────────────────────────────────────┐      │
│  │  OAuth2 (Casdoor / Bohrium) ─── Token Validation          │      │
│  │  Material CRUD (Web + Edge) ─── Material WebSocket        │      │
│  │  gRPC Services (4) ─── Auth Interceptor                   │      │
│  │  SSE Notifications ─── Health Checks ─── Swagger UI       │      │
│  └───────────────────────────────────────────────────────────┘      │
│                                                                     │
│  ┌─ Schedule Server ────────────────────────────────────────┐      │
│  │  WebSocket Hub (Melody, 200 conns)                        │      │
│  │  Task Queue Consumer (BRPop) ─── Control Queue Consumer   │      │
│  │  DAG Engine ─── Notebook Engine ─── Action Engine         │      │
│  │  Heartbeat Monitor ─── Device Status Broadcast            │      │
│  └───────────────────────────────────────────────────────────┘      │
│                                                                     │
│  Edge Devices ←──── WebSocket :8081 ────→ Physical Lab Hardware    │
└─────────────────────────────────────────────────────────────────────┘
```
