# OSDL — Open Science Device Lab

**OSDL** is a decoupled, open-source communication infrastructure for managing and orchestrating scientific laboratory Edge devices. Upstream applications (e.g. Studio, third-party systems) interact with Edge devices through OSDL's gRPC / HTTP API, while Edge software like [Uni-Lab-Edge](https://github.com/Uni-Lab-Edge/unilabos) (a DeepModeling community project) connects to OSDL from the device side. OSDL handles the full lifecycle: WebSocket connections, real-time messaging, task scheduling (DAG / Notebook / Action), material graph management, and pluggable authentication via [Casdoor](https://casdoor.org/) or [Bohrium](https://bohrium.dp.tech/).

> **Language / 语言**: English | [中文文档](./docs/README_CN.md)

---

## Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                        Upstream Applications                        │
│                      (Studio, Third-party Apps)                     │
└──────────┬──────────────────────────────────┬────────────────────────┘
           │  gRPC (port 9090)                │  HTTP REST (port 8080)
           ▼                                  ▼
┌──────────────────────────────────────────────────────────────────────┐
│                          OSDL API Server                            │
│                                                                      │
│  ┌─────────────┐  ┌──────────────┐  ┌────────────┐  ┌────────────┐ │
│  │ Auth Module  │  │Material CRUD │  │ SSE Notify │  │ gRPC Layer │ │
│  │(Casdoor/Bohr)│  │   & Graph    │  │  (Events)  │  │  Services  │ │
│  └──────┬──────┘  └──────┬───────┘  └─────┬──────┘  └──────┬─────┘ │
│         │                │                 │                │       │
│         ▼                ▼                 ▼                ▼       │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                      Core Business Layer                     │   │
│  │    Login ─── Material ─── Schedule ─── Notify (Broadcast)    │   │
│  └──────────────────────────────────┬───────────────────────────┘   │
│                                     │                               │
│  ┌──────────────────────────────────┴───────────────────────────┐   │
│  │                      Repository Layer                        │   │
│  │  Casdoor Repo ─ Bohrium Repo ─ Material Repo ─ Env Repo     │   │
│  │    Sandbox Repo ─── Migrate                                  │   │
│  └──────────────────────────────────┬───────────────────────────┘   │
└──────────────────────────────────────┼──────────────────────────────┘
                                       │
           ┌───────────────────────────┼───────────────────────────┐
           ▼                           ▼                           ▼
    ┌─────────────┐           ┌──────────────┐       ┌─────────────────┐
    │  PostgreSQL  │           │    Redis     │       │ Casdoor / Bohr  │
    │  (persist)   │           │ (queue/pub)  │       │  (Auth Backend) │
    └─────────────┘           └──────────────┘       └─────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│                       OSDL Schedule Server                          │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐  │
│  │                    WebSocket Hub (Melody)                      │  │
│  │                                                                │  │
│  │   Edge A ←──ws──→  Control  ←──ws──→  Edge B                  │  │
│  └───────────┬──────────────────────────────────┬────────────────┘  │
│              │                                  │                    │
│   ┌──────────▼──────────┐          ┌────────────▼─────────────┐     │
│   │  Task Queue (Redis) │          │  Control Queue (Redis)   │     │
│   │   BRPop consumer    │          │    BRPop consumer        │     │
│   └──────────┬──────────┘          └────────────┬─────────────┘     │
│              │                                  │                    │
│   ┌──────────▼──────────────────────────────────▼─────────────┐     │
│   │                   Execution Engine                        │     │
│   │     DAG Workflow ─── Notebook ─── Single Action           │     │
│   └───────────────────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────────────┐
│                    Edge Devices (Uni-Lab-Edge)                       │
│            github.com/Uni-Lab-Edge/unilabos (DeepModeling)          │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐                    │
│  │  Edge Dev 1 │  │  Edge Dev 2 │  │  Edge Dev N │                    │
│  └──────┬─────┘  └──────┬─────┘  └──────┬─────┘                    │
│         └───────────┬────┘───────────────┘                          │
│                     │ WebSocket (:8081)                              │
└─────────────────────┼───────────────────────────────────────────────┘
                      ▼
              OSDL Schedule Server
```

### System Architecture (Mermaid)

```mermaid
graph TB
    subgraph Upstream["Upstream Applications"]
        ST[Studio]
        TP[Third-party Apps]
    end

    subgraph OSDL["OSDL"]
        direction TB
        subgraph API["API Server :8080 / :9090"]
            HTTP[HTTP REST API]
            GRPC[gRPC Services]
            AUTH["Auth (Casdoor / Bohrium)"]
            MAT[Material CRUD]
            SSE[SSE Notifications]
        end

        subgraph SCH["Schedule Server :8081"]
            WS[WebSocket Hub]
            TQ[Task Queue Consumer]
            CQ[Control Queue Consumer]
            ENG[Engine: DAG / Notebook / Action]
        end

        subgraph Core["Core Layer"]
            LOGIN[Login Service]
            MATCORE[Material Service]
            SCHED[Schedule Control]
            NOTIFY[Broadcast - Redis Pub/Sub]
        end

        subgraph Repo["Repository Layer"]
            CASREPO[Casdoor Client]
            BOHRREPO[Bohrium Client]
            MATREPO[Material Repo]
            ENVREPO[Environment Repo]
            SANDREPO[Sandbox Repo]
        end
    end

    subgraph Infra["Infrastructure"]
        PG[(PostgreSQL)]
        RD[(Redis)]
        CAS[Casdoor / Bohrium]
        SB[Sandbox]
    end

    subgraph Edge["Edge Devices (Uni-Lab-Edge)"]
        E1[Edge Device 1]
        E2[Edge Device 2]
    end

    ST & TP -->|gRPC / HTTP| API
    API --> Core
    SCH --> Core
    Core --> Repo
    Repo --> PG & RD & CAS & SB
    E1 & E2 <-->|WebSocket| WS
    WS --> TQ & CQ
    TQ & CQ --> ENG
    NOTIFY -.->|Redis Pub/Sub| SSE
```

---

## Sequence Diagrams

### 1. Edge Device Connection & Task Execution

```mermaid
sequenceDiagram
    participant Edge as Edge Device
    participant WS as Schedule Server<br/>(WebSocket)
    participant Redis as Redis
    participant Engine as Execution Engine

    Edge->>WS: WebSocket connect<br/>(Lab AK/SK auth)
    WS->>WS: Auth middleware validates AK/SK
    WS->>Redis: SetNX lab heartbeat key
    Redis-->>WS: OK
    WS->>WS: Create EdgeImpl instance
    WS->>Redis: SetEx heartbeat (periodic)
    WS-->>Edge: Connection established

    Edge->>WS: {"action":"host_node_ready"}
    WS->>WS: onEdgeReady()
    WS->>Redis: BRPop task queue (loop)
    WS->>Redis: BRPop control queue (loop)

    Note over WS,Redis: ─── Upstream sends a task ───

    Redis-->>WS: Task message from queue
    WS->>Engine: onJobMessage → Start DAG/Notebook/Action
    Engine->>Edge: Send action commands via session.Write()

    Edge->>WS: {"action":"report_action_state", ...}
    WS->>Engine: SetDeviceActionStatus()
    Engine->>Engine: Check completion → next step

    Edge->>WS: {"action":"job_status", ...}
    WS->>Engine: OnJobUpdate()
    Engine-->>WS: Task complete

    Note over Edge,WS: ─── Heartbeat keeps alive ───

    loop Every LabHeartTime
        WS->>Redis: SetEx heartbeat TTL
    end

    Edge->>WS: {"action":"normal_exit"}
    WS->>WS: EdgeImpl.Close()
    WS->>Redis: Del heartbeat key
```

### 2. OAuth2 Login Flow (Casdoor)

```mermaid
sequenceDiagram
    participant Client as Browser / App
    participant API as OSDL API Server
    participant Redis as Redis
    participant CAS as Casdoor

    Client->>API: GET /api/auth/login?frontend_callback_url=...
    API->>API: Generate random state
    API->>Redis: Set state → frontend_callback_url (5min TTL)
    API-->>Client: 302 Redirect → Casdoor authorize URL

    Client->>CAS: User login on Casdoor
    CAS-->>Client: 302 Redirect → /api/auth/callback/casdoor?code=xxx&state=yyy

    Client->>API: GET /api/auth/callback/casdoor?code=xxx&state=yyy
    API->>Redis: Get frontend_callback_url by state
    Redis-->>API: frontend_callback_url
    API->>CAS: Exchange code → access_token + refresh_token
    CAS-->>API: Token response
    API->>CAS: GET /api/get-account (with token)
    CAS-->>API: User info
    API-->>Client: 302 Redirect → frontend_callback_url?access_token=...

    Note over Client,API: ─── Token refresh ───

    Client->>API: POST /api/auth/refresh {refresh_token}
    API->>CAS: Refresh token
    CAS-->>API: New token pair
    API-->>Client: {access_token, refresh_token, expires_in}
```

### 3. Material Sync (Edge ↔ Platform)

```mermaid
sequenceDiagram
    participant Edge as Edge Device
    participant API as OSDL API Server
    participant DB as PostgreSQL
    participant WS as Schedule Server
    participant FE as Frontend (SSE)

    Note over Edge,API: ─── Edge reports material topology ───

    Edge->>API: POST /api/v1/edge/material/create<br/>(Lab AK/SK auth)
    API->>DB: Insert material nodes
    DB-->>API: Created
    API-->>Edge: Material items

    Edge->>API: POST /api/v1/edge/material/upsert
    API->>DB: Upsert nodes + data
    DB-->>API: Updated
    API-->>Edge: OK

    Note over WS,FE: ─── Real-time device status ───

    Edge->>WS: {"action":"device_status", "device_id":"pump-1", ...}
    WS->>DB: UpdateMaterialNodeDataKey()
    WS->>WS: Broadcast via Redis Pub/Sub
    WS-->>FE: SSE event: material_modify

    Note over FE,API: ─── Frontend queries material ───

    FE->>API: GET /api/v1/lab/material/query?lab_uuid=...
    API->>DB: Select material graph
    DB-->>API: Nodes + edges
    API-->>FE: Material DAG JSON
```

### 4. gRPC Service Call Flow

```mermaid
sequenceDiagram
    participant App as Upstream App
    participant GRPC as OSDL gRPC Server
    participant Core as Core Layer
    participant Redis as Redis
    participant WS as Schedule Server
    participant Edge as Edge Device

    App->>GRPC: ScheduleService.StartAction(lab_uuid, device, action)
    GRPC->>Core: Validate + build task
    Core->>Redis: LPush task to lab queue
    Redis-->>Core: OK
    GRPC-->>App: {task_uuid}

    Redis-->>WS: BRPop picks up task
    WS->>Edge: Send action command

    App->>GRPC: EdgeService.StreamDeviceStatus(lab_uuid)
    GRPC->>GRPC: Subscribe Redis Pub/Sub
    loop Device status updates
        Edge->>WS: Device status
        WS->>Redis: Publish status
        Redis-->>GRPC: Status event
        GRPC-->>App: stream DeviceStatusEvent
    end
```

---

## Project Structure

```
osdl/
├── main.go                          # Entry point — Cobra CLI (apiserver / schedule / migrate)
├── go.mod
├── Makefile                         # Build, dev, docker, lint targets
├── Dockerfile                       # Multi-stage build (alpine)
├── docker-compose.yml               # Full stack: postgres + redis + osdl-api + osdl-schedule
├── .env.example                     # Environment variable template
│
├── cmd/
│   ├── api/server.go                # API server startup (HTTP + gRPC + graceful shutdown)
│   └── schedule/server.go           # Schedule server startup (WebSocket + Redis consumer)
│
├── internal/config/                 # Configuration (env vars via Viper)
│
├── proto/osdl/v1/                   # gRPC Proto definitions
│   ├── edge.proto                   # EdgeService — device status & streaming
│   ├── schedule.proto               # ScheduleService — workflow/notebook/action
│   ├── material.proto               # MaterialService — material CRUD
│   └── auth.proto                   # AuthService — OAuth2 login/callback/refresh
│
├── pkg/
│   ├── common/                      # Shared: UUID, error codes, constants, response
│   ├── core/
│   │   ├── login/casdoor/           # Casdoor OAuth2 implementation
│   │   ├── material/                # Material business logic + Edge sync
│   │   ├── schedule/
│   │   │   ├── control/             # WebSocket hub (Melody) + connection lifecycle
│   │   │   ├── lab/edge/            # EdgeImpl — message routing & queue consumers
│   │   │   └── engine/              # Task execution: DAG, Notebook, Action
│   │   └── notify/events/           # Redis Pub/Sub broadcast system
│   ├── grpc/                        # gRPC server bootstrap
│   ├── middleware/                   # Auth, DB, Redis, Logger, OpenTelemetry
│   ├── repo/                        # Repository interfaces + implementations
│   │   ├── casdoor/                 # Casdoor auth backend
│   │   └── bohr/                    # Bohrium auth backend
│   ├── utils/                       # DAG, JWT, signal, concurrency helpers
│   └── web/                         # Gin HTTP routes + handlers
│       └── views/                   # health, login, material, schedule, sse
│
└── gen/osdl/v1/                     # protoc-generated Go code (gitignored or tracked)
```

---

## Quick Start

### Prerequisites

- Go 1.24+
- PostgreSQL 16+
- Redis 7+
- [Casdoor](https://casdoor.org/) instance (for OAuth2, default) **or** [Bohrium](https://bohrium.dp.tech/) account (set `OAUTH_SOURCE=bohr`)

### Local Development

```bash
# 1. Clone
git clone https://github.com/ScienceOL/OSDL.git && cd OSDL

# 2. Copy env and configure
cp .env.example .env
# Edit .env with your database, Redis, and Casdoor settings

# 3. Install dependencies
make init

# 4. Run database migration
make migrate

# 5. Start API server (HTTP :8080 + gRPC :9090)
make apiserver

# 6. Start Schedule server (WebSocket :8081) — in another terminal
make schedule
```

### Docker Compose (one command)

```bash
# Start everything: PostgreSQL + Redis + migrate + API + Schedule
make docker-up

# View logs
make docker-logs

# Stop
make docker-down
```

---

## API Endpoints

### Health Checks

| Method | Path                | Description                          |
|--------|---------------------|--------------------------------------|
| GET    | `/api/health`       | Basic health check                   |
| GET    | `/api/health/live`  | Liveness probe (always OK)           |
| GET    | `/api/health/ready` | Readiness probe (checks PG + Redis)  |

### Authentication

| Method | Path                           | Description                    |
|--------|--------------------------------|--------------------------------|
| GET    | `/api/auth/login`              | Initiate Casdoor OAuth2 login  |
| GET    | `/api/auth/callback/casdoor`   | OAuth2 callback                |
| POST   | `/api/auth/refresh`            | Refresh access token           |

### Material Management (Bearer auth)

| Method | Path                                     | Description                |
|--------|------------------------------------------|----------------------------|
| POST   | `/api/v1/lab/material/create`            | Create lab material        |
| POST   | `/api/v1/lab/material/save`              | Save material              |
| GET    | `/api/v1/lab/material/query`             | Query materials            |
| PUT    | `/api/v1/lab/material/update`            | Batch update               |
| GET    | `/api/v1/lab/material/download/:lab_uuid`| Download material graph    |

### Edge Device API (Lab AK/SK auth)

| Method | Path                                | Description                  |
|--------|-------------------------------------|------------------------------|
| POST   | `/api/v1/edge/material/create`      | Edge creates material nodes  |
| POST   | `/api/v1/edge/material/upsert`      | Edge upserts material        |
| POST   | `/api/v1/edge/material/edge`        | Edge creates connections     |
| GET    | `/api/v1/edge/material/download`    | Edge downloads material DAG  |

### WebSocket

| Path                                  | Server   | Description                     |
|---------------------------------------|----------|---------------------------------|
| `/api/v1/ws/material/:lab_uuid`       | API      | Material real-time updates      |
| `/api/v1/ws/schedule`                 | Schedule | Edge device ↔ OSDL connection   |

### SSE

| Path                      | Description                        |
|---------------------------|------------------------------------|
| `/api/v1/lab/notify/sse`  | Server-Sent Events for broadcasts  |

### gRPC Services (port 9090)

| Service           | Methods                                                          |
|-------------------|------------------------------------------------------------------|
| `EdgeService`     | `GetEdgeStatus`, `StreamDeviceStatus`                            |
| `ScheduleService` | `StartWorkflow`, `StartNotebook`, `StartAction`, `StopJob`, `StreamJobStatus` |
| `MaterialService` | `EdgeCreateMaterial`, `EdgeUpsertMaterial`, `EdgeCreateEdge`, `QueryMaterial`, `DownloadMaterial` |
| `AuthService`     | `Login`, `Callback`, `Refresh`                                   |

---

## Configuration

All configuration is via environment variables (loaded from `.env`):

| Variable               | Default         | Description                        |
|------------------------|-----------------|------------------------------------|
| `DATABASE_HOST`        | `localhost`     | PostgreSQL host                    |
| `DATABASE_PORT`        | `5432`          | PostgreSQL port                    |
| `DATABASE_NAME`        | `osdl`          | Database name                      |
| `DATABASE_USER`        | `postgres`      | Database user                      |
| `DATABASE_PASSWORD`    | `osdl`          | Database password                  |
| `REDIS_HOST`           | `127.0.0.1`    | Redis host                         |
| `REDIS_PORT`           | `6379`          | Redis port                         |
| `WEB_PORT`             | `8080`          | HTTP API port                      |
| `SCHEDULE_PORT`        | `8081`          | Schedule WebSocket port            |
| `GRPC_PORT`            | `9090`          | gRPC port                          |
| `OAUTH2_CLIENT_ID`     | —               | Casdoor OAuth2 client ID           |
| `OAUTH2_CLIENT_SECRET` | —               | Casdoor OAuth2 client secret       |
| `CASDOOR_ADDR`         | —               | Casdoor server address             |
| `OAUTH_SOURCE`         | `casdoor`       | Auth backend (`casdoor` or `bohr`) |
| `BOHR_CORE_ADDR`       | —               | Bohrium Core API address           |
| `ACCOUNT_ADDR`         | —               | Bohrium Account API address        |
| `BOHR_ADDR`            | —               | Bohrium API address                |
| `SANDBOX_ADDR`         | —               | Sandbox service address            |
| `LOG_LEVEL`            | `info`          | Log level (debug/info/warn/error)  |
| `ENV`                  | `dev`           | Environment (dev/prod)             |

See [`.env.example`](./.env.example) for the complete list.

---

## Make Targets

```bash
make help          # Show all available commands
make init          # Download and tidy dependencies
make apiserver     # Run API server
make schedule      # Run Schedule server
make migrate       # Run database migration
make build         # Build binary
make build-linux   # Cross-compile for Linux
make proto         # Generate gRPC code from proto files
make test          # Run tests
make fmt           # Format code
make vet           # Go vet
make lint          # Lint (golangci-lint)
make docker-build  # Build Docker image
make docker-up     # Start full stack with docker-compose
make docker-down   # Stop all services
make docker-logs   # Tail logs
make clean         # Clean build artifacts
```

---

## Tech Stack

| Component        | Technology                                   |
|------------------|----------------------------------------------|
| Language         | Go 1.24                                      |
| HTTP Framework   | [Gin](https://github.com/gin-gonic/gin)      |
| WebSocket        | [Melody](https://github.com/olahol/melody)   |
| gRPC             | [gRPC-Go](https://google.golang.org/grpc)    |
| ORM              | [GORM](https://gorm.io/) + PostgreSQL        |
| Cache / Queue    | [Redis](https://redis.io/) (go-redis/v9)     |
| Authentication   | [Casdoor](https://casdoor.org/) or [Bohrium](https://bohrium.dp.tech/) |
| CLI              | [Cobra](https://github.com/spf13/cobra)      |
| Config           | [Viper](https://github.com/spf13/viper)      |
| Logging          | [Zap](https://github.com/uber-go/zap)        |
| Tracing          | [OpenTelemetry](https://opentelemetry.io/)    |
| Goroutine Pool   | [ants](https://github.com/panjf2000/ants)    |
| Container        | Docker + Docker Compose                       |

---

## License

[AGPL-3.0](./LICENSE)
