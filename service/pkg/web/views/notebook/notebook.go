package notebook

import (
	// 外部依赖
	"context"

	gin "github.com/gin-gonic/gin"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	nbCore "github.com/scienceol/opensdl/service/pkg/core/notebook"
	impl "github.com/scienceol/opensdl/service/pkg/core/notebook/notebook"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
)

type Handle struct {
	nbService nbCore.Service
}

func NewNotebookHandle(ctx context.Context) *Handle {
	return &Handle{
		nbService: impl.New(ctx),
	}
}

// QueryNotebook 查询 notebook 列表
func (h *Handle) QueryNotebook(ctx *gin.Context) {
	req := &nbCore.QueryNotebookReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "QueryNotebook parse param err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	resp, err := h.nbService.QueryNotebookList(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "QueryNotebook err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

// CreateNotebook 创建 notebook
func (h *Handle) CreateNotebook(ctx *gin.Context) {
	req := &nbCore.CreateNotebookReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "CreateNotebook parse param err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	resp, err := h.nbService.CreateNotebook(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "CreateNotebook err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

// DeleteNotebook 删除 notebook
func (h *Handle) DeleteNotebook(ctx *gin.Context) {
	req := &nbCore.DeleteNotebookReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		logger.Errorf(ctx, "DeleteNotebook parse uri param err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	if err := h.nbService.DeleteNotebook(ctx, req); err != nil {
		logger.Errorf(ctx, "DeleteNotebook err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx)
}

func (h *Handle) NotebookSchema(ctx *gin.Context) {
	req := &nbCore.NotebookSchemaReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "NotebookSchema parse uri param err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	resp, err := h.nbService.NotebookSchema(ctx, req)
	common.Reply(ctx, err, resp)
}

// NotebookDetail 获取 notebook 详情
func (h *Handle) NotebookDetail(ctx *gin.Context) {
	req := &nbCore.NotebookDetailReq{}
	// 优先读取 query 参数（兼容 ?uuid=xxx）
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "NotebookDetail bind query ignore err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithMsg("uuid is required"))
		return
	}

	resp, err := h.nbService.NotebookDetail(ctx, req)
	common.Reply(ctx, err, resp)
}

// 创建样品
func (h *Handle) CreateSample(ctx *gin.Context) {
	req := &nbCore.SampleReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "CreateSample bind query err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	resp, err := h.nbService.CreateSample(ctx, req)
	common.Reply(ctx, err, resp)
}

// GetNotebookBySample 根据样品获取 notebook
func (h *Handle) GetNotebookBySample(ctx *gin.Context) {
	req := &nbCore.GetNotebookBySampleReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	resp, err := h.nbService.GetNotebookBySample(ctx, req)
	if err != nil {
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}
