package model

import (
	// 内部引用
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
)

type Sample struct {
	BaseModel
	LabID          int64  `gorm:"type:bigint;not null;index:idx_sample_lnm,priority:1" json:"lab_id"`
	MaterialNodeID int64  `gorm:"type:bigint;not null;index:idx_sample_lnm,priority:2" json:"material_node_id"`
	Name           string `gorm:"type:varchar(255);not null;index:idx_sample_lnm,priority:3" json:"name"`

	MaterialNodeUUID uuid.UUID `gorm:"-"`
}

func (*Sample) TableName() string {
	return "sample"
}
