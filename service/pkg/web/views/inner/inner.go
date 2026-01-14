package inner

import (
	// 外部依赖
	gin "github.com/gin-gonic/gin"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	inner "github.com/scienceol/opensdl/service/pkg/core/inner"
	impl "github.com/scienceol/opensdl/service/pkg/core/inner/inner"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
)

/*
	内部模块需要基本的 basic auth 认证
*/

type Handle struct {
	service inner.Service
}

func New() *Handle {
	return &Handle{
		service: impl.NewInner(),
	}
}

/*定义一个配置自定义权限获取方法*/
func (h *Handle) GetUserCustomPolicy(ctx *gin.Context) {
	req := &inner.CustomPolicyReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := h.service.GetUserCustomPolicy(ctx, req)
	common.Reply(ctx, err, resp)
}

func (h *Handle) GetPolicyResource(ctx *gin.Context) {
	resp, err := h.service.GetResources(ctx)
	common.Reply(ctx, err, resp)
}
