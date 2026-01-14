package storage

import (
	// 外部依赖
	gin "github.com/gin-gonic/gin"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	storage "github.com/scienceol/opensdl/service/pkg/core/storage"
	st "github.com/scienceol/opensdl/service/pkg/core/storage/storage"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
)

// StorageHandle 提供 Storage 相关接口
type StorageHandle struct {
	storageService storage.StorageService
}

func NewStorageHandle() *StorageHandle {
	storageSvc := st.NewStorageService()
	return &StorageHandle{storageService: storageSvc}
}

// CreateStoragePathToken 获取Storage路径token
// GET /api/v1/applications/token?scene=xxx&filename=xxx.txt
func (s *StorageHandle) CreateStoragePathToken(ctx *gin.Context) {
	req := &storage.GetStorageTokenReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	if !req.Scene.IsValid() {
		logger.Infof(ctx, "scene is invalid: %+v", req.Scene)
		req.Scene = repo.SceneDefault
	}

	resp, err := s.storageService.GenerateStorageToken(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "CreateLaboratoryEnv err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}
