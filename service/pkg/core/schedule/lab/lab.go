package lab

import (
	// 外部依赖
	"context"

	melody "github.com/olahol/melody"
)

type Edge interface {
	// edge 侧发送消息
	OnEdgeMessge(ctx context.Context, s *melody.Session, b []byte)
	// 心跳消息
	OnPongMessage(ctx context.Context)
	// 处理关闭逻辑
	Close(ctx context.Context)
}
