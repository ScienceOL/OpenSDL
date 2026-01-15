package repo

import (
	// 外部依赖
	"context"

	// 内部引用
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type SceneType string

const (
	SceneJob     SceneType = "job"
	SceneDefault SceneType = "default"
)

func (s SceneType) IsValid() bool {
	switch s {
	case SceneDefault, SceneJob:
		return true
	default:
		return false
	}
}

type StorageTokenRepo interface {
	Create(ctx context.Context, data *model.StorageToken) error
}
