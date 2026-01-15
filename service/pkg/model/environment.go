package model

import (
	// 外部依赖
	"time"

	datatypes "gorm.io/datatypes"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
)

type EnvironmentStatus string

const (
	INIT    EnvironmentStatus = "init"
	DELETED EnvironmentStatus = "deleted"
)

// 实验室环境表
type Laboratory struct {
	BaseModel
	Name         string            `gorm:"type:varchar(120);not null;index:idx_laboratory_user_status_name,priority:3" json:"name"`
	UserID       string            `gorm:"type:varchar(120);not null;index:idx_laboratory_user_status_name,priority:1" json:"user_id"`
	Status       EnvironmentStatus `gorm:"type:varchar(20);not null;index:idx_laboratory_user_status_name,priority:2" json:"status"`
	AccessKey    string            `gorm:"type:varchar(120);not null;uniqueIndex:idx_laboratory_lab_id_ak_sk,priority:1" json:"access_key"`
	AccessSecret string            `gorm:"type:varchar(120);not null;uniqueIndex:idx_laboratory_lab_id_ak_sk,priority:2" json:"access_secret"`
	Description  *string           `gorm:"type:text" json:"description"`
}

func (*Laboratory) TableName() string {
	return "laboratory"
}

type LaboratoryMember struct {
	BaseModel
	UserID  string      `gorm:"type:varchar(120);not null;uniqueIndex:idx_laboratorymemeber_lu,priority:1" json:"user_id"`
	LabID   int64       `gorm:"type:bigint;not null;index:idx_laboratorymemeber_ld;uniqueIndex:idx_laboratorymemeber_lu,priority:2" json:"lab_id"`
	Role    common.Role `gorm:"type:varchar(120);not null" json:"role"`
	PinTime *time.Time  `gorm:"default:null" json:"pin_time"`
}

func (*LaboratoryMember) TableName() string {
	return "laboratory_member"
}

type InvitationType string

const (
	InvitationTypeLab InvitationType = "lab"
)

type LaboratoryInvitation struct {
	BaseModel
	ExpiresAt time.Time                  `gorm:"not null;default:CURRENT_TIMESTAMP" json:"expires_at"`
	Type      InvitationType             `gorm:"type:varchar(50);not null;index:idx_labinv_tt,priority:1" json:"type"`
	ThirdID   string                     `gorm:"type:varchar(50);not null;index:idx_labinv_tt,priority:2" json:"third_id"`
	UserID    string                     `gorm:"type:varchar(120);not null" json:"user_id"`
	RoleIDs   datatypes.JSONSlice[int64] `gorm:"type:jsonb" json:"role_ids"`
}

func (*LaboratoryInvitation) TableName() string {
	return "laboratory_invitation"
}

type Project struct {
	BaseModel
	LabID       int64   `gorm:"type:bigint;not null;index:idx_project_ln,priority:1" json:"lab_id"`
	Name        string  `gorm:"type:varchar(120);not null;index:idx_project_ln,priority:2" json:"name"`
	Description *string `gorm:"type:text" json:"description"`
}

func (*Project) TableName() string {
	return "project"
}

type ProjectMember struct {
	BaseModel
	LabID     int64  `gorm:"type:bigint;not null;uniqueIndex:idx_projectmember_lpu,priority:1" json:"lab_id"`
	ProjectID int64  `gorm:"type:bigint;not null;uniqueIndex:idx_projectmember_lpu,priority:2" json:"project_id"`
	UserID    string `gorm:"type:varchar(120);not null;uniqueIndex:idx_projectmember_lpu,priority:3;index:idx_projectmember_projectid_userid,priority:2" json:"user_id"`
}

func (*ProjectMember) TableName() string {
	return "project_member"
}
