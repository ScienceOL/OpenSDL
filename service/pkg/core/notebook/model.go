package notebook

import (
	// 外部依赖
	"time"

	datatypes "gorm.io/datatypes"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	constant "github.com/scienceol/opensdl/service/pkg/common/constant"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

// QueryNotebookReq 查询 notebook 列表请求
type QueryNotebookReq struct {
	LabUUID              uuid.UUID              `json:"lab_uuid" form:"lab_uuid" uri:"lab_uuid"`
	Search               string                 `json:"search" form:"search"`   // 搜索关键词（按名称搜索）
	Status               []model.NotebookStatus `json:"status" form:"status[]"` // 状态
	SubmitTime           *constant.SortOrder    `json:"submit_time" form:"submit_time"`
	ScheduleStartTime    *constant.SortOrder    `json:"start_time" form:"start_time"`
	ScheduleFinishedTime *constant.SortOrder    `json:"end_time" form:"finished_time"`
	common.PageReq
}

// QueryNotebookResp 查询 notebook 列表响应
type QueryNotebookResp struct {
	common.PageResp[[]*NotebookItem] `json:",inline"`
}

// NotebookItem notebook 列表项
type NotebookItem struct {
	UUID         uuid.UUID            `json:"uuid"`
	Name         string               `json:"name"`
	Status       model.NotebookStatus `json:"status"`
	UserID       string               `json:"user_id"`
	UserName     string               `json:"user_name"`    // 作者名称
	DisplayName  string               `json:"display_name"` // 作者显示名称
	SubmitTime   time.Time            `json:"submit_time"`
	StartTime    *time.Time           `json:"start_time"`    // 如果为零值则返回 null
	FinishedTime *time.Time           `json:"finished_time"` // 如果为零值则返回 null
	CreatedAt    time.Time            `json:"created_at"`
	UpdatedAt    time.Time            `json:"updated_at"`
}

// QueryNotebookGroupReq 查询 notebook_group 列表请求
type QueryNotebookGroupReq struct {
	NotebookUUID uuid.UUID `json:"notebook_uuid" form:"notebook_uuid" uri:"notebook_uuid" binding:"required"`
	common.PageReq
}

// QueryNotebookGroupResp 查询 notebook_group 列表响应
type QueryNotebookGroupResp struct {
	common.PageMoreResp[[]*NotebookGroupItem] `json:",inline"`
}

// NotebookGroupItem notebook_group 列表项
type NotebookGroupItem struct {
	UUID         uuid.UUID            `json:"uuid"`
	NotebookUUID uuid.UUID            `json:"notebook_uuid"`
	Status       model.NotebookStatus `json:"status"`
	StartTime    time.Time            `json:"start_time"`
	FinishedTime time.Time            `json:"finished_at"`
	CreatedAt    time.Time            `json:"created_at"`
	UpdatedAt    time.Time            `json:"updated_at"`
}

type WellData struct{}

type NotebookParam struct {
	NodeUUID uuid.UUID      `json:"node_uuid" binding:"required"`
	Param    datatypes.JSON `json:"param" binding:"required"`
	// SampleParams []model.SampleParam `json:"sample_params,omitempty"`
}

type NotebookTask struct {
	SampleUUIDs []uuid.UUID      `json:"sample_uuids"`
	Datas       []*NotebookParam `json:"datas" binding:"required"`
}

// CreateNotebookReq 创建 notebook 请求
type CreateNotebookReq struct {
	LabUUID      uuid.UUID       `json:"lab_uuid" binding:"required"`
	WorkflowUUID uuid.UUID       `json:"workflow_uuid" binding:"required"`
	Name         string          `json:"name" binding:"required"`
	NodeParams   []*NotebookTask `json:"node_params" binding:"required"`
}

// CreateNotebookResp 创建 notebook 响应
type CreateNotebookResp struct {
	UUID uuid.UUID `json:"uuid"`
}

// CreateNotebookGroupReq 创建 notebook_group 请求
type CreateNotebookGroupReq struct {
	NotebookUUID uuid.UUID `json:"notebook_uuid" binding:"required"`
}

// CreateNotebookGroupResp 创建 notebook_group 响应
type CreateNotebookGroupResp struct {
	UUID uuid.UUID `json:"uuid"`
}

// DeleteNotebookReq 删除 notebook 请求
type DeleteNotebookReq struct {
	UUID uuid.UUID `json:"uuid" form:"uuid" uri:"uuid" binding:"required"`
}

// DeleteNotebookGroupReq 删除 notebook_group 请求
type DeleteNotebookGroupReq struct {
	NotebookGroupUUID uuid.UUID `json:"notebook_group_uuid" binding:"required"`
}

// 获取工作流 schema 请求
type NotebookSchemaReq struct {
	UUID uuid.UUID `json:"uuid" form:"uuid" uri:"uuid" binding:"required"`
}

type NodeSchema struct {
	UUID   uuid.UUID      `json:"uuid"`
	Name   string         `json:"name"`
	Schema datatypes.JSON `json:"schema"`
	Param  datatypes.JSON `json:"param"`
}

type NotebookSchemaResp struct {
	UUID        uuid.UUID     `json:"uuid"`
	NodeSchemas []*NodeSchema `json:"node_schemas"`
}

type SchemaGoal struct {
	Properties map[string]any `json:"properties"`
}

type SchemaProperty struct {
	Goal SchemaGoal `json:"goal"`
}

type NodeTemplateSchema struct {
	Properties SchemaProperty `json:"properties"`
}

// 运行实验记录本请求
type NotebookRunReq struct {
	UUID uuid.UUID `json:"uuid"`
}

// NotebookDetailReq 获取 notebook 详情请求
type NotebookDetailReq struct {
	UUID uuid.UUID `json:"uuid" form:"uuid" uri:"uuid" binding:"required"`
}

type NotebookDetailItem struct {
	NodeUUID     uuid.UUID                              `json:"node_uuid"`
	Param        datatypes.JSON                         `json:"param,omitempty"`
	SampleParams datatypes.JSONSlice[model.SampleParam] `json:"sample_params"`
	Result       *datatypes.JSONType[model.ReturnInfo]  `json:"result,omitempty"`
}

type SampleInfo struct {
	SampleUUID uuid.UUID `json:"sample_uuid"`
	Name       string    `json:"name"`
}

type NotebookGroup struct {
	Samples []*SampleInfo         `json:"samples"`
	Params  []*NotebookDetailItem `json:"params"`
}

type NotebookDetailResp struct {
	UUID           uuid.UUID            `json:"uuid"`            // notebook UUID
	Name           string               `json:"name"`            // 实验名称
	Status         model.NotebookStatus `json:"status"`          // 实验状态
	WorkflowUUID   uuid.UUID            `json:"workflow_uuid"`   // 工作流 UUID
	UserName       string               `json:"user_name"`       // 作者名称
	DisplayName    string               `json:"display_name"`    // 作者显示名称
	NotebookGroups []*NotebookGroup     `json:"notebook_groups"` // 工作流详情
}

type SampleItem struct {
	MaterialNodeUUID uuid.UUID `json:"material_node_uuid" binding:"required"`
	Name             string    `json:"name" binding:"required"`
	SampleUUID       uuid.UUID `json:"sample_uuid"`
}

type SampleReq struct {
	LabUUID uuid.UUID     `json:"lab_uuid" binding:"required"`
	Items   []*SampleItem `json:"items"`
}

type SampleResp struct {
	Items []*SampleItem `json:"items"`
}

// QueryNotebookReq 查询 notebook 列表请求
type GetNotebookBySampleReq struct {
	SampleName string `form:"sample_name" binding:"required"`
}

// GetNotebookBySampleResp 根据样品获取 notebook 响应
type GetNotebookBySampleResp struct {
	Res []*NotebookBySampleItem `json:"res"`
}

type SampleData struct {
	ReturnInfo        []*model.ReturnInfo `json:"return_info"`
	NotebookGroupUUID uuid.UUID           `json:"notebook_group_uuid"`
}

// NotebookBySampleItem 根据样品获取的 notebook 项
type NotebookBySampleItem struct {
	NotebookUUID uuid.UUID     `json:"notebook_uuid"`
	SampleData   []*SampleData `json:"sample_data"`
}
