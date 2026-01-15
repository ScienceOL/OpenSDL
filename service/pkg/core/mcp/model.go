package mcp

import (
	// 外部依赖
	datatypes "gorm.io/datatypes"

	// 内部引用
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type TaskStatusReq struct {
	UUID uuid.UUID `json:"uuid" uri:"uuid" form:"uuid" binding:"required"`
}

type TaskJobStatus struct {
	UUID       uuid.UUID                            `json:"uuid"`
	NodeName   string                               `json:"node_name"`
	ActionName string                               `json:"action_name"`
	Status     string                               `json:"status"`
	ReturnInfo datatypes.JSONType[model.ReturnInfo] `json:"return_info"`
}

type TaskStatusResp struct {
	Status    string           `json:"status"`
	JosStatus []*TaskJobStatus `json:"jos_status"`
}
