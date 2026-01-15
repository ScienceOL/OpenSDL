package model

import (
	// 外部依赖
	"time"

	datatypes "gorm.io/datatypes"

	// 内部引用
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
)

/*
 实验记录本相关，一个实验记录本记录是一组 task 组成，包含多个不同的工作流任务。
*/

type NotebookStatus string

const (
	NotebookStatusInit    NotebookStatus = "init"
	NotebookStatusPending NotebookStatus = "pending"
	NotebookStatusRunnig  NotebookStatus = "running"
	NotebookStatusSuccess NotebookStatus = "success"
	NotebookStatusFail    NotebookStatus = "fail"
	NotebookStatusDeleted NotebookStatus = "deleted"
)

// 关联一批实验记录
type Notebook struct {
	BaseModel
	LabID        int64          `gorm:"type:bigint;not null;index:idx_notebook_lpuw,priority:1" json:"lab_id"`
	ProjectID    int64          `gorm:"type:bigint;not null;default:0;index:idx_notebook_lpuw,priority:2" json:"project_id"`
	WorkflowID   int64          `gorm:"type:bigint;not null;index:idx_notebook_lpuw,priority:4" json:"workflow_id"`
	UserID       string         `gorm:"type:varchar(120);not null;index:idx_notebook_lpuw,priority:3" json:"user_id"`
	Name         string         `gorm:"type:varchar(120);not null;index:idx_notebook_lpuw,priority:5" json:"name"`
	Status       NotebookStatus `gorm:"type:varchar(50);not null;default:'init'" json:"status"`
	SubmitTime   time.Time      `gorm:"column:submit_time" json:"submit_time"`
	StartTime    time.Time      `gorm:"column:start_time" json:"start_time"`
	FinishedTime time.Time      `gorm:"column:finished_time" json:"finished_time"`
	DeletedAt    *time.Time     `gorm:"column:deleted_at" json:"deleted_at"`
}

func (*Notebook) TableName() string {
	return "notebook"
}

// 记录本组实验记录
type NotebookGroup struct {
	BaseModel
	NotebookID   int64                          `gorm:"type:bigint;not null;index:idx_notebookgroup_n,priority:1" json:"notebook_id"`
	Status       NotebookStatus                 `gorm:"type:varchar(50);not null;default:'init'" json:"status"`
	StartTime    time.Time                      `gorm:"column:start_time" json:"start_time"`
	FinishedTime time.Time                      `gorm:"column:finished_time" json:"finished_time"`
	SampleUUIDs  datatypes.JSONSlice[uuid.UUID] `gorm:"type:jsonb" json:"sample_uuids"`
}

func (*NotebookGroup) TableName() string {
	return "notebook_group"
}

type SampleParam struct {
	ContainerUUID uuid.UUID      `json:"container_uuid"` // 容器设备的 uuid
	SampleValue   map[string]any `json:"sample_value"`
}

// 一组实验记录下的一组参数
type NotebookParam struct {
	BaseModel
	NotebookGroupID int64                            `gorm:"type:bigint;not null" json:"notebook_group_id"`
	WorkflowNodeID  int64                            `gorm:"type:bigint;not null" json:"workflow_node_id"`
	Param           datatypes.JSON                   `gorm:"type:jsonb" json:"param"`
	SampleValue     datatypes.JSONSlice[SampleParam] `gorm:"type:jsonb" json:"sample_value"`
}

func (*NotebookParam) TableName() string {
	return "notebook_param"
}
