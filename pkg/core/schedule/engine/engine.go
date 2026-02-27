package engine

import (
	"context"
	"time"

	"github.com/scienceol/osdl/pkg/common/uuid"
)

type Task interface {
	Run() error
	Stop() error
	GetStatus(ctx context.Context) error
	OnJobUpdate(ctx context.Context, data *JobData) error
	Type(ctx context.Context) JobType
	ID(ctx context.Context) uuid.UUID
	GetDeviceActionStatus(ctx context.Context, key ActionKey) (ActionValue, bool)
	SetDeviceActionStatus(ctx context.Context, key ActionKey, free bool, needMore time.Duration)
	InitDeviceActionStatus(ctx context.Context, key ActionKey, start time.Time, free bool)
	DelStatus(ctx context.Context, key ActionKey)
}
