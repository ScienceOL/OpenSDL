package notebook

import (
	"context"
	"time"

	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/core/schedule/engine"
)

func NewNotebookTask(_ context.Context, _ *engine.NotebookInfo) engine.Task {
	return &notebookEngine{}
}

type notebookEngine struct{}

func (n *notebookEngine) Run() error                        { return nil }
func (n *notebookEngine) Stop() error                       { return nil }
func (n *notebookEngine) GetStatus(_ context.Context) error { return nil }
func (n *notebookEngine) OnJobUpdate(_ context.Context, _ *engine.JobData) error {
	return nil
}
func (n *notebookEngine) Type(_ context.Context) engine.JobType { return engine.NotebookJobType }
func (n *notebookEngine) ID(_ context.Context) uuid.UUID        { return uuid.NewNil() }
func (n *notebookEngine) GetDeviceActionStatus(_ context.Context, _ engine.ActionKey) (engine.ActionValue, bool) {
	return engine.ActionValue{}, false
}
func (n *notebookEngine) SetDeviceActionStatus(_ context.Context, _ engine.ActionKey, _ bool, _ time.Duration) {
}
func (n *notebookEngine) InitDeviceActionStatus(_ context.Context, _ engine.ActionKey, _ time.Time, _ bool) {
}
func (n *notebookEngine) DelStatus(_ context.Context, _ engine.ActionKey) {}
