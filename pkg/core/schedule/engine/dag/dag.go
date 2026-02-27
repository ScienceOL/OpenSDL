package dag

import (
	"context"
	"time"

	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/core/schedule/engine"
)

func NewDagTask(_ context.Context, _ *engine.TaskParam) engine.Task {
	return &dagEngine{}
}

type dagEngine struct{}

func (d *dagEngine) Run() error                        { return nil }
func (d *dagEngine) Stop() error                       { return nil }
func (d *dagEngine) GetStatus(_ context.Context) error { return nil }
func (d *dagEngine) OnJobUpdate(_ context.Context, _ *engine.JobData) error {
	return nil
}
func (d *dagEngine) Type(_ context.Context) engine.JobType { return engine.WorkflowJobType }
func (d *dagEngine) ID(_ context.Context) uuid.UUID        { return uuid.NewNil() }
func (d *dagEngine) GetDeviceActionStatus(_ context.Context, _ engine.ActionKey) (engine.ActionValue, bool) {
	return engine.ActionValue{}, false
}
func (d *dagEngine) SetDeviceActionStatus(_ context.Context, _ engine.ActionKey, _ bool, _ time.Duration) {
}
func (d *dagEngine) InitDeviceActionStatus(_ context.Context, _ engine.ActionKey, _ time.Time, _ bool) {
}
func (d *dagEngine) DelStatus(_ context.Context, _ engine.ActionKey) {}
