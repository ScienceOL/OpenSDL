package schedule

import (
	"context"
	"github.com/scienceol/osdl/pkg/core/schedule/engine"
	"github.com/scienceol/osdl/pkg/repo/model"
)

const LABINFO = "LAB_INFO"

type ActionType string

const (
	JobStart          ActionType = "job_start"
	QueryActionStatus ActionType = "query_action_state"
	Pong              ActionType = "pong"
	CancelTask        ActionType = "cancel_task"
	JobStatus         ActionType = "job_status"
	DeviceStatus      ActionType = "device_status"
	Ping              ActionType = "ping"
	ReportActionState ActionType = "report_action_state"
)

type LabInfo struct {
	LabUser *model.UserData
	LabData *model.Laboratory
}

type ControlTask struct {
	Task   engine.Task
	Cancle context.CancelFunc
	Ctx    context.Context
}
