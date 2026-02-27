package schedule

import "context"

type Control interface {
	Connect(ctx context.Context)
	Close(ctx context.Context)
}
