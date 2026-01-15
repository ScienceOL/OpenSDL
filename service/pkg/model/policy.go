package model

import (
	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
)

// 策略资源
type PolicyResource struct {
	BaseModel
	Name        string `gorm:"type:varchar(160);not null;uniqueIndex:idx_r_rn,priority:1" json:"name"`
	Description string `gorm:"type:text;default ''" json:"description"`
}

func (*PolicyResource) TableName() string {
	return "policy_resource"
}

// 角色
type CustomRole struct {
	BaseModel
	LabID       int64  `gorm:"type:bigint;not null;uniqueIndex:idx_cr_lr,priority:1" json:"lab_id"`
	RoleName    string `gorm:"type:varchar(100);not null;uniqueIndex:idx_cr_lr,priority:2" json:"role_name"`
	Description string `gorm:"type:text;default ''" json:"description"`
}

func (*CustomRole) TableName() string {
	return "custom_role"
}

// 角色权限关系
type CustomRolePerm struct {
	BaseModel
	CustomRoleID     int64       `gorm:"type:bigint;not null;uniqueIndex:idx_crp_crp,priority:1" json:"custom_role_id"`
	PolicyResourceID int64       `gorm:"type:bigint;not null;uniqueIndex:idx_crp_crp,priority:2" json:"policy_resource_id"`
	Perm             common.Perm `gorm:"type:varchar(60);not null;uniqueIndex:idx_crp_crp,priority:3" json:"perm"`
}

func (*CustomRolePerm) TableName() string {
	return "custom_role_perm"
}

// 用户角色关系
type UserRole struct {
	BaseModel
	LabID        int64  `gorm:"type:bigint;not null;uniqueIndex:idx_ur_luc,priority:1" json:"lab_id"`
	UserID       string `gorm:"type:varchar(120);not null;uniqueIndex:idx_ur_luc,priority:2" json:"user_id"`
	CustomRoleID int64  `gorm:"type:bigint;not null;uniqueIndex:idx_ur_luc,priority:3" json:"custom_role_id"`
}

func (*UserRole) TableName() string {
	return "user_role"
}
