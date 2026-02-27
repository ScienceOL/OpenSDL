package model

import (
	"github.com/scienceol/osdl/pkg/common/uuid"
	"gorm.io/datatypes"
)

type MaterialNode struct {
	BaseModel
	LabID        int64          `gorm:"not null;index" json:"lab_id"`
	ParentID     *int64         `json:"parent_id"`
	TemplateID   *int64         `json:"template_id"`
	Name         string         `gorm:"not null" json:"name"`
	UniqueName   string         `json:"unique_name"`
	ResourceType string         `json:"resource_type"`
	Data         datatypes.JSON `json:"data"`
	Position     datatypes.JSON `json:"position"`
	Config       datatypes.JSON `json:"config"`
	Status       string         `json:"status"`
}

type MaterialEdge struct {
	BaseModel
	LabID        int64     `gorm:"not null;index" json:"lab_id"`
	SourceID     uuid.UUID `gorm:"type:uuid" json:"source_id"`
	TargetID     uuid.UUID `gorm:"type:uuid" json:"target_id"`
	SourceHandle string    `json:"source_handle"`
	TargetHandle string    `json:"target_handle"`
}

type ResourceNodeTemplate struct {
	BaseModel
	LabID        int64          `gorm:"not null;uniqueIndex:idx_lab_name" json:"lab_id"`
	Name         string         `gorm:"not null;uniqueIndex:idx_lab_name" json:"name"`
	Header       datatypes.JSON `json:"header"`
	Footer       datatypes.JSON `json:"footer"`
	Icon         string         `json:"icon"`
	Description  string         `json:"description"`
	Model        string         `json:"model"`
	Module       string         `json:"module"`
	ResourceType string         `json:"resource_type"`
	Language     string         `json:"language"`
	StatusTypes  datatypes.JSON `json:"status_types"`
	Tags         datatypes.JSON `json:"tags"`
	DataSchema   datatypes.JSON `json:"data_schema"`
	ConfigSchema datatypes.JSON `json:"config_schema"`
	Pose         datatypes.JSON `json:"pose"`
	Version      string         `json:"version"`
	ConfigInfo   datatypes.JSON `json:"config_info"`
}

type ResourceHandleTemplate struct {
	ID             int64  `gorm:"primaryKey" json:"id"`
	ResourceNodeID int64  `gorm:"not null;uniqueIndex:idx_res_handle" json:"resource_node_id"`
	Name           string `gorm:"not null;uniqueIndex:idx_res_handle" json:"name"`
	IoType         string `gorm:"not null;uniqueIndex:idx_res_handle" json:"io_type"`
	Side           string `gorm:"not null;uniqueIndex:idx_res_handle" json:"side"`
	DisplayName    string `json:"display_name"`
	Type           string `json:"type"`
	Source         string `json:"source"`
	Key            string `json:"key"`
}
