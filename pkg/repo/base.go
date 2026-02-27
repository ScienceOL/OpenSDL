package repo

import "github.com/scienceol/osdl/pkg/common/uuid"

type IDOrUUIDTranslate interface {
	GetIDByUUID(uuid uuid.UUID) (int64, error)
	GetUUIDByID(id int64) (uuid.UUID, error)
}
