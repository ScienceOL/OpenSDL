package mcp

import (
	// 外部依赖
	gin "github.com/gin-gonic/gin"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	s "github.com/scienceol/opensdl/service/pkg/core/mcp"
	mcp "github.com/scienceol/opensdl/service/pkg/core/mcp/mcp"
	action "github.com/scienceol/opensdl/service/pkg/core/schedule/engine/action"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
)

type Handle struct {
	srv s.Service
}

func NewHandle() *Handle {
	return &Handle{
		srv: mcp.New(),
	}
}

func (h *Handle) RunAction(ctx *gin.Context) {
	req := &action.RunActionReq{}
	if err := ctx.BindJSON(req); err != nil {
		logger.Errorf(ctx, "parse RunAction param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := h.srv.RunAction(ctx, req)
	common.Reply(ctx, err, resp)
}

// 查询 task 运行状态
func (h *Handle) Task(ctx *gin.Context) {
	req := &s.TaskStatusReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	res, err := h.srv.QueryTaskStatus(ctx, req)
	common.Reply(ctx, err, res)
}
