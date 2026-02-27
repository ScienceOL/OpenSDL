package lab

import (
	"context"
	"github.com/olahol/melody"
)

type Edge interface {
	OnEdgeMessge(ctx context.Context, s *melody.Session, b []byte)
	OnPongMessage(ctx context.Context)
	Close(ctx context.Context)
}
