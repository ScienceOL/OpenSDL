package reagent

import (
	// 外部依赖
	"github.com/gin-gonic/gin"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	coreReagent "github.com/scienceol/opensdl/service/pkg/core/reagent"
	reagentImpl "github.com/scienceol/opensdl/service/pkg/core/reagent/reagent"
)

type Handle struct{ svc coreReagent.Service }

func NewHandle() *Handle { return &Handle{svc: reagentImpl.New()} }

func (h *Handle) Insert(ctx *gin.Context) {
	in := &coreReagent.InsertReq{}
	if err := ctx.ShouldBindJSON(in); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}

	resp, err := h.svc.Insert(ctx, in)
	common.Reply(ctx, err, resp)
}

func (h *Handle) Query(ctx *gin.Context) {
	in := &coreReagent.QueryReq{}
	if err := ctx.ShouldBindQuery(in); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	resp, err := h.svc.Query(ctx, in)
	common.Reply(ctx, err, resp)
}

func (h *Handle) Delete(ctx *gin.Context) {
	in := &coreReagent.DeleteReq{}
	if err := ctx.ShouldBindJSON(in); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	common.Reply(ctx, h.svc.Delete(ctx, in))
}

func (h *Handle) Update(ctx *gin.Context) {
	reqs := &coreReagent.UpdateReq{}
	if err := ctx.ShouldBindJSON(&reqs); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	common.Reply(ctx, h.svc.Update(ctx, reqs))
}

func (h *Handle) QueryCAS(ctx *gin.Context) {
	in := &coreReagent.CasReq{}
	if err := ctx.ShouldBindQuery(in); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	resp, err := h.svc.QueryCAS(ctx, in)
	common.Reply(ctx, err, resp)
}
