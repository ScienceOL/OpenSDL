package model

import (
	"time"

	"gorm.io/datatypes"
)

type EnvironmentStatus string

const (
	EnvironmentActive  EnvironmentStatus = "active"
	EnvironmentDeleted EnvironmentStatus = "deleted"
)

type Laboratory struct {
	BaseModel
	Name            string            `gorm:"not null" json:"name"`
	UserID          string            `gorm:"not null;index" json:"user_id"`
	Status          EnvironmentStatus `gorm:"default:active" json:"status"`
	AccessKey       string            `gorm:"uniqueIndex" json:"access_key"`
	AccessSecret    string            `gorm:"uniqueIndex" json:"access_secret"`
	IsOnline        bool              `gorm:"default:false" json:"is_online"`
	LastConnectedAt *time.Time        `json:"last_connected_at"`
	Settings        datatypes.JSON    `json:"settings,omitempty"`
	Description     string            `json:"description,omitempty"`
}

type LaboratoryMember struct {
	BaseModel
	LabID   int64      `gorm:"not null;uniqueIndex:idx_lab_user" json:"lab_id"`
	UserID  string     `gorm:"not null;uniqueIndex:idx_lab_user" json:"user_id"`
	PinTime *time.Time `json:"pin_time,omitempty"`
}

type CustomRole struct {
	ID       int64  `gorm:"primaryKey" json:"id"`
	LabID    int64  `gorm:"not null" json:"lab_id"`
	RoleName string `gorm:"not null" json:"role_name"`
}

type UserRole struct {
	ID           int64  `gorm:"primaryKey" json:"id"`
	LabID        int64  `gorm:"not null" json:"lab_id"`
	UserID       string `gorm:"not null" json:"user_id"`
	CustomRoleID int64  `gorm:"not null" json:"custom_role_id"`
}

type LaboratoryInvitation struct {
	BaseModel
	LabID     int64     `gorm:"not null" json:"lab_id"`
	ExpiresAt time.Time `json:"expires_at"`
}
