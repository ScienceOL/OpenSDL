# ================================
# Stage 1: Build
# ================================
FROM golang:1.24-alpine AS builder

RUN apk add --no-cache git ca-certificates tzdata

WORKDIR /src

# Cache dependencies
COPY go.mod go.sum ./
RUN go mod download

# Copy source and build
COPY . .

ARG VERSION=dev
ARG GIT_COMMIT=unknown
ARG BUILD_TIME=unknown

RUN CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build \
    -ldflags="-s -w -X main.Version=${VERSION} -X main.BuildTime=${BUILD_TIME} -X main.GitCommit=${GIT_COMMIT}" \
    -o /bin/osdl .

# ================================
# Stage 2: Runtime
# ================================
FROM alpine:3.21

RUN apk add --no-cache ca-certificates tzdata wget \
    && cp /usr/share/zoneinfo/Asia/Shanghai /etc/localtime \
    && echo "Asia/Shanghai" > /etc/timezone \
    && apk del tzdata

RUN addgroup -S osdl && adduser -S -G osdl osdl

COPY --from=builder /bin/osdl /usr/local/bin/osdl

USER osdl

# Default ports: HTTP 8080, Schedule 8081, gRPC 9090
EXPOSE 8080 8081 9090

HEALTHCHECK --interval=15s --timeout=3s --start-period=10s --retries=3 \
    CMD wget --quiet --tries=1 --spider http://localhost:8080/api/health/live || exit 1

ENTRYPOINT ["osdl"]
CMD ["apiserver"]
