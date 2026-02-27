package engine

import (
	"time"

	"github.com/olahol/melody"
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/repo"
	"github.com/scienceol/osdl/pkg/repo/model"
	"gorm.io/datatypes"
)

const (
	DataKeySplit = "@@@"
)

type JobType string

const (
	WorkflowJobType JobType = "workflow"
	NotebookJobType JobType = "notebook"
	ActionJobType   JobType = "action"
)

type WorkflowInfo struct {
	TaskUUID     uuid.UUID `json:"task_uuid"`
	WorkflowUUID uuid.UUID `json:"workflow_id"` // 任务 id

	LabUUID uuid.UUID `json:"lab_uuid"`
	UserID  string    `json:"user_id"` // 提交用户 id
	LabID   int64     `json:"lab_id"`  // 实验室 id

	TaskID int64 `json:"-"`
}

type NotebookInfo struct {
	Session *melody.Session
	Sandbox repo.Sandbox

	NotebookUUID uuid.UUID `json:"task_uuid"`
	LabUUID      uuid.UUID `json:"lab_uuid"` // 实验室 uuid
	LabID        int64     `json:"lab_id"`   // 实验室 id
	UserID       string    `json:"user_id"`  // 提交用户 id

	// WorkflowUUID uuid.UUID
	// TaskUUID     uuid.UUID `json:"-"`
	// TaskID       int64     `json:"-"`
	WorkflowID int64 `json:"-"`
	NotebookID int64 `json:"-"`
}

type TaskParam struct {
	Session      *melody.Session
	Sandbox      repo.Sandbox
	WorkflowInfo *WorkflowInfo
}

type NotifyEdge struct{}

type ActionParam struct {
	Session      *melody.Session
	Sandbox      repo.Sandbox
	WorkflowInfo *WorkflowInfo
}

type WorkflowAction string

const (
	StartJob       WorkflowAction = "start_job"
	StopJob        WorkflowAction = "stop_job"
	StatusJob      WorkflowAction = "status_job"
	StartAction    WorkflowAction = "start_action"
	AddMaterial    WorkflowAction = "add_material"
	UpdateMaterial WorkflowAction = "update_material"
	RemoveMaterial WorkflowAction = "remove_material"
)

type ServerInfo struct {
	SendTimestamp float64 `json:"send_timestamp"`
}

type SendActionData struct {
	DeviceID       string                  `json:"device_id"`
	Action         string                  `json:"action"`
	ActionType     string                  `json:"action_type"`
	ActionArgs     datatypes.JSON          `json:"action_args"`
	JobID          uuid.UUID               `json:"job_id"`
	TaskID         uuid.UUID               `json:"task_id"`
	NodeID         uuid.UUID               `json:"node_id"`
	ServerInfo     ServerInfo              `json:"server_info"`
	SampleMaterial map[uuid.UUID]uuid.UUID `json:"sample_material"`
}

type BoardMsg struct {
	NodeUUID    uuid.UUID                            `json:"node_uuid"`    // 节点 uuid
	TaskStatus  string                               `json:"task_status"`  // 工作流状态
	JobStatus   string                               `json:"job_status"`   // 节点状态
	Header      string                               `json:"header"`       // action 名
	Type        string                               `json:"type"`         // 日志级别
	Msg         string                               `json:"msg"`          // 消息体
	StackTrace  []string                             `json:"stack_trace"`  // 错误堆栈信息
	ReturnInfos datatypes.JSONType[model.ReturnInfo] `json:"return_infos"` // 返回结果
	Timestamp   time.Time                            `json:"timestamp"`    // 日志时间戳
}

type CancelTask struct {
	TaskID uuid.UUID `json:"task_id"`
}

type JobData struct {
	JobID      uuid.UUID `json:"job_id"`
	TaskID     uuid.UUID `json:"task_id"`
	DeviceID   string    `json:"device_id"`
	ActionName string    `json:"action_name"`

	Status       string                               `json:"status"`
	FeedbackData datatypes.JSON                       `json:"feedback_data"`
	ReturnInfo   datatypes.JSONType[model.ReturnInfo] `json:"return_info"`
	// FIXME: 数字字符串，无法转换
	// Timestamp    time.Time      `json:"timestamp"`
}

type HandlePair struct {
	SourceHandle *model.WorkflowHandleTemplate
	TargetHandle *model.WorkflowHandleTemplate
	SourceNode   *model.WorkflowNode
}

type StatusType string

const (
	QueryActionStatus StatusType = "query_action_status"
	JobCallbackStatus StatusType = "job_call_back_status"
)

type ActionKey struct {
	Type       StatusType `json:"type"`
	TaskUUID   uuid.UUID  `json:"task_id"`
	JobID      uuid.UUID  `json:"job_id"`
	DeviceID   string     `json:"device_id"`
	ActionName string     `json:"action_name"`
}

type ActionValue struct {
	Free      bool          `json:"free"`
	NeedMore  time.Duration `json:"need_more"`
	Timestamp time.Time     `json:"timestamp"`
}

type MaterialUpdate struct {
	UUID          uuid.UUID `json:"uuid"`
	DeviceOldUUID uuid.UUID `json:"device_old_uuid"`
	DeviceOldID   string    `json:"device_old_id"`
	DeviceUUID    uuid.UUID `json:"device_uuid"`
	DeviceID      string    `json:"device_id"`
}
