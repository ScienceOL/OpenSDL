# ========================
# OSDL Makefile
# ========================

PROJECT_NAME := osdl
MODULE_NAME := github.com/scienceol/osdl
BINARY_NAME := osdl
BINARY_DIR := bin
CMD_DIR := ./

GO := go
GOOS := $(shell go env GOOS)
GOARCH := $(shell go env GOARCH)
CGO_ENABLED := 0

VERSION := $(shell git describe --tags --always --dirty 2>/dev/null || echo "dev")
BUILD_TIME := $(shell date -u '+%Y-%m-%d_%H:%M:%S')
GIT_COMMIT := $(shell git rev-parse --short HEAD 2>/dev/null || echo "unknown")

LDFLAGS := -ldflags="-s -w -X main.Version=$(VERSION) -X main.BuildTime=$(BUILD_TIME) -X main.GitCommit=$(GIT_COMMIT)"

.DEFAULT_GOAL := help

.PHONY: help
help: ## Show help
	@echo "OSDL - Open Science Device Lab"
	@echo ""
	@echo "Available commands:"
	@awk 'BEGIN {FS = ":.*?## "} /^[a-zA-Z_-]+:.*?## / {printf "  \033[36m%-15s\033[0m %s\n", $$1, $$2}' $(MAKEFILE_LIST)

# ===== Development =====

.PHONY: init
init: ## Initialize dependencies
	$(GO) mod download
	$(GO) mod tidy

.PHONY: dev
dev: ## Run API server with hot-reload (air)
	@if ! command -v air > /dev/null 2>&1; then \
		echo "Installing air (hot-reload)..."; \
		$(GO) install github.com/air-verse/air@v1.62.0; \
	fi
	air -c .air.web.toml

.PHONY: dev-schedule
dev-schedule: ## Run Schedule server with hot-reload (air)
	@if ! command -v air > /dev/null 2>&1; then \
		echo "Installing air (hot-reload)..."; \
		$(GO) install github.com/air-verse/air@v1.62.0; \
	fi
	air -c .air.schedule.toml

.PHONY: apiserver
apiserver: ## Run API server
	$(GO) run $(CMD_DIR) apiserver

.PHONY: schedule
schedule: ## Run Schedule server
	$(GO) run $(CMD_DIR) schedule

.PHONY: migrate
migrate: ## Run database migration
	$(GO) run $(CMD_DIR) migrate

.PHONY: swagger
swagger: ## Generate Swagger documentation
	@if ! command -v swag > /dev/null 2>&1; then \
		echo "Installing swag..."; \
		$(GO) install github.com/swaggo/swag/cmd/swag@latest; \
	fi
	swag init -g main.go

# ===== Build =====

.PHONY: build
build: clean ## Build binary
	@mkdir -p $(BINARY_DIR)
	CGO_ENABLED=$(CGO_ENABLED) GOOS=$(GOOS) GOARCH=$(GOARCH) \
	$(GO) build $(LDFLAGS) -o $(BINARY_DIR)/$(BINARY_NAME) $(CMD_DIR)

.PHONY: build-linux
build-linux: clean ## Build for Linux
	@mkdir -p $(BINARY_DIR)
	CGO_ENABLED=$(CGO_ENABLED) GOOS=linux GOARCH=amd64 \
	$(GO) build $(LDFLAGS) -o $(BINARY_DIR)/$(BINARY_NAME)-linux $(CMD_DIR)

# ===== Proto =====

.PHONY: proto
proto: ## Generate gRPC code from proto files
	@rm -rf gen/osdl/v1/*.pb.go
	protoc --go_out=. --go_opt=paths=import \
		--go-grpc_out=. --go-grpc_opt=paths=import \
		proto/osdl/v1/*.proto
	@if [ -d "github.com/scienceol/osdl/gen/osdl/v1" ]; then \
		cp github.com/scienceol/osdl/gen/osdl/v1/*.go gen/osdl/v1/ && \
		rm -rf github.com; \
	fi

# ===== Quality =====

.PHONY: test
test: ## Run tests
	$(GO) test -v ./...

.PHONY: fmt
fmt: ## Format code
	$(GO) fmt ./...
	@if command -v goimports > /dev/null; then goimports -w ./pkg ./cmd ./internal; fi

.PHONY: vet
vet: ## Go vet
	$(GO) vet ./...

.PHONY: lint
lint: fmt ## Lint code
	@if command -v golangci-lint > /dev/null; then golangci-lint run -v --timeout=5m ./...; fi

.PHONY: mod
mod: ## Tidy dependencies
	$(GO) mod tidy
	$(GO) mod verify

# ===== Clean =====

.PHONY: clean
clean: ## Clean build artifacts
	@rm -rf $(BINARY_DIR)
	@rm -f coverage.out coverage.html

# ===== Docker =====

DOCKER_IMAGE := osdl
DOCKER_TAG := $(VERSION)

.PHONY: docker-build
docker-build: ## Build Docker image
	docker build --build-arg VERSION=$(VERSION) --build-arg GIT_COMMIT=$(GIT_COMMIT) --build-arg BUILD_TIME=$(BUILD_TIME) \
		-t $(DOCKER_IMAGE):$(DOCKER_TAG) -t $(DOCKER_IMAGE):latest .

.PHONY: docker-up
docker-up: ## Start all services with docker-compose
	docker compose up -d --build

.PHONY: docker-down
docker-down: ## Stop all services
	docker compose down

.PHONY: docker-logs
docker-logs: ## Tail logs from all services
	docker compose logs -f
