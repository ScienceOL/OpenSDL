package model

import (
	"time"

	"github.com/scienceol/osdl/pkg/common/uuid"
	"gorm.io/datatypes"
)

type WorkflowNodeTemplate struct {
	BaseModel
	LabID          int64          `json:"lab_id"`
	ResourceNodeID int64          `gorm:"uniqueIndex:idx_res_name" json:"resource_node_id"`
	Name           string         `gorm:"uniqueIndex:idx_res_name" json:"name"`
	Class          string         `json:"class"`
	Goal           datatypes.JSON `json:"goal"`
	GoalDefault    datatypes.JSON `json:"goal_default"`
	Feedback       datatypes.JSON `json:"feedback"`
	Result         datatypes.JSON `json:"result"`
	Schema         datatypes.JSON `json:"schema"`
	Type           string         `json:"type"`
	Icon           string         `json:"icon"`
	DisplayName    string         `json:"display_name"`
}

type WorkflowHandleTemplate struct {
	ID             int64  `gorm:"primaryKey" json:"id"`
	WorkflowNodeID int64  `gorm:"not null;uniqueIndex:idx_wf_handle" json:"workflow_node_id"`
	HandleKey      string `gorm:"not null;uniqueIndex:idx_wf_handle" json:"handle_key"`
	IoType         string `gorm:"not null;uniqueIndex:idx_wf_handle" json:"io_type"`
	DisplayName    string `json:"display_name"`
	Type           string `json:"type"`
	DataSource     string `json:"data_source"`
	DataKey        string `json:"data_key"`
}

type WorkflowTask struct {
	BaseModel
	WorkflowID int64          `json:"workflow_id"`
	LabID      int64          `json:"lab_id"`
	UserID     string         `json:"user_id"`
	Status     string         `json:"status"`
	StartedAt  *time.Time     `json:"started_at"`
	FinishedAt *time.Time     `json:"finished_at"`
	Error      string         `json:"error"`
	Result     datatypes.JSON `json:"result"`
}

type Workflow struct {
	BaseModel
	LabID       int64          `json:"lab_id"`
	Name        string         `json:"name"`
	Description string         `json:"description"`
	UserID      string         `json:"user_id"`
	Status      string         `json:"status"`
	Graph       datatypes.JSON `json:"graph"`
	Config      datatypes.JSON `json:"config"`
}

type WorkflowNode struct {
	BaseModel
	WorkflowID int64          `json:"workflow_id"`
	LabID      int64          `json:"lab_id"`
	Name       string         `json:"name"`
	Type       string         `json:"type"`
	DeviceName string         `json:"device_name"`
	ActionName string         `json:"action_name"`
	ActionType string         `json:"action_type"`
	Position   datatypes.JSON `json:"position"`
	Data       datatypes.JSON `json:"data"`
	Config     datatypes.JSON `json:"config"`
}

type WorkflowEdge struct {
	BaseModel
	WorkflowID   int64     `json:"workflow_id"`
	SourceNodeID uuid.UUID `gorm:"type:uuid" json:"source_node_id"`
	TargetNodeID uuid.UUID `gorm:"type:uuid" json:"target_node_id"`
	SourceHandle string    `json:"source_handle"`
	TargetHandle string    `json:"target_handle"`
}

type WorkflowNodeJob struct {
	BaseModel
	TaskID       int64          `json:"task_id"`
	NodeID       int64          `json:"node_id"`
	WorkflowID   int64          `json:"workflow_id"`
	LabID        int64          `json:"lab_id"`
	Status       string         `json:"status"`
	StartedAt    *time.Time     `json:"started_at"`
	FinishedAt   *time.Time     `json:"finished_at"`
	Error        string         `json:"error"`
	FeedbackData datatypes.JSON `json:"feedback_data"`
	ReturnInfo   datatypes.JSON `json:"return_info"`
}

type ReturnInfo struct {
	Type  string `json:"type"`
	Value any    `json:"value"`
}

type NotebookGroup struct {
	BaseModel
	NotebookID int64          `json:"notebook_id"`
	LabID      int64          `json:"lab_id"`
	Status     string         `json:"status"`
	Config     datatypes.JSON `json:"config"`
}

type NotebookParam struct {
	ID         int64          `gorm:"primaryKey" json:"id"`
	GroupID    int64          `json:"group_id"`
	NotebookID int64          `json:"notebook_id"`
	NodeID     int64          `json:"node_id"`
	Key        string         `json:"key"`
	Value      datatypes.JSON `json:"value"`
}

type NotebookSample struct {
	BaseModel
	GroupID    int64          `json:"group_id"`
	NotebookID int64          `json:"notebook_id"`
	LabID      int64          `json:"lab_id"`
	Data       datatypes.JSON `json:"data"`
}
