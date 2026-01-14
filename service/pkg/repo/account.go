package repo

import (
	// 外部依赖
	"context"

	// 内部引用
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type Account interface {
	// 创建用户
	// CreateLabUser(ctx context.Context, user *model.LabInfo) error
	// 批量获取用户信息
	BatchGetUserInfo(ctx context.Context, uesrIDs []string) ([]*model.UserData, error)
	// 获取指定用户信息
	GetUserInfo(ctx context.Context, userID string) (*model.UserData, error)
	// 根据 ak、sk 获取实验室创建者信息
	GetLabUserInfo(ctx context.Context, req *model.LabAkSk) (*model.UserData, error)
	// 根据账户 ak 获取用户信息，bohr 使用
	GetLabUserByAccessKey(ctx context.Context, accessKey string) (*model.UserData, error)
}
