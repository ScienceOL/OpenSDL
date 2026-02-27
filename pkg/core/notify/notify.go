package notify

import (
	"context"

	"github.com/scienceol/osdl/pkg/common/uuid"
)

type Action string

const (
	MaterialModify Action = "material-modify"
	WorkflowRun    Action = "workflow-run"
	MsgNotify      Action = "msg-notify"
)

type SendMsg struct {
	Channel      Action    `json:"action"`
	LabUUID      uuid.UUID `json:"lab_uuid"`
	WorkflowUUID uuid.UUID `json:"work_flow_uud"`
	TaskUUID     uuid.UUID `json:"task_uuid"`
	UserID       string    `json:"user_id"`
	Data         any       `json:"data"`
	UUID         uuid.UUID `json:"uuid"`
	Timestamp    int64     `json:"timestamp"`
}

type HandleFunc func(ctx context.Context, msg string) error

type MsgCenter interface {
	Registry(ctx context.Context, msgName Action, handleFunc HandleFunc) error
	Broadcast(ctx context.Context, msg *SendMsg) error
	Close(ctx context.Context) error
}
