package model

import (
	"time"

	"github.com/scienceol/osdl/pkg/common/uuid"
)

type BaseModel struct {
	ID        int64     `gorm:"primaryKey" json:"id"`
	UUID      uuid.UUID `gorm:"type:uuid;uniqueIndex" json:"uuid"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}
