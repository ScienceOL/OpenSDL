package action

import (
	"context"
	"time"

	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/core/schedule/engine"
)

func NewActionTask(_ context.Context, _ *engine.ActionParam) engine.Task {
	return &actionEngine{}
}

type actionEngine struct{}

func (a *actionEngine) Run() error                        { return nil }
func (a *actionEngine) Stop() error                       { return nil }
func (a *actionEngine) GetStatus(_ context.Context) error { return nil }
func (a *actionEngine) OnJobUpdate(_ context.Context, _ *engine.JobData) error {
	return nil
}
func (a *actionEngine) Type(_ context.Context) engine.JobType { return engine.ActionJobType }
func (a *actionEngine) ID(_ context.Context) uuid.UUID        { return uuid.NewNil() }
func (a *actionEngine) GetDeviceActionStatus(_ context.Context, _ engine.ActionKey) (engine.ActionValue, bool) {
	return engine.ActionValue{}, false
}
func (a *actionEngine) SetDeviceActionStatus(_ context.Context, _ engine.ActionKey, _ bool, _ time.Duration) {
}
func (a *actionEngine) InitDeviceActionStatus(_ context.Context, _ engine.ActionKey, _ time.Time, _ bool) {
}
func (a *actionEngine) DelStatus(_ context.Context, _ engine.ActionKey) {}
