package storage_token

import (
	// 外部依赖
	"context"

	// 内部引用
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type storageTokenImpl struct {
	repo.IDOrUUIDTranslate
}

func New() repo.StorageTokenRepo {
	return &storageTokenImpl{
		IDOrUUIDTranslate: repo.NewBaseDB(),
	}
}

func (n *storageTokenImpl) Create(ctx context.Context, data *model.StorageToken) error {
	if err := n.DBWithContext(ctx).Create(data).Error; err != nil {
		logger.Errorf(ctx, "Create storage_token fail err: %+v", err)
		return code.CreateDataErr.WithMsg(err.Error())
	}
	return nil
}
