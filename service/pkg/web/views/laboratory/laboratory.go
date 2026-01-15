package laboratory

import (
	// 外部依赖
	gin "github.com/gin-gonic/gin"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	environment "github.com/scienceol/opensdl/service/pkg/core/environment"
	laboratory "github.com/scienceol/opensdl/service/pkg/core/environment/laboratory"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
)

type EnvHandle struct {
	envService environment.EnvService
}

func NewEnvironment() *EnvHandle {
	return &EnvHandle{
		envService: laboratory.NewLab(),
	}
}

func (l *EnvHandle) CreateLabEnv(ctx *gin.Context) {
	req := &environment.LaboratoryEnvReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.CreateLaboratoryEnv(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "CreateLaboratoryEnv err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) UpdateLabEnv(ctx *gin.Context) {
	req := &environment.UpdateEnvReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.UpdateLaboratoryEnv(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "CreateLaboratoryEnv err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) DelLabEnv(ctx *gin.Context) {
	req := &environment.DelLabReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	err := l.envService.DelLab(ctx, req)
	common.Reply(ctx, err)
}

func (l *EnvHandle) LabList(ctx *gin.Context) {
	req := &common.PageReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.LabList(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "LabList err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) LabInfo(ctx *gin.Context) {
	req := &environment.LabInfoReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.LabInfo(ctx, req)
	common.Reply(ctx, err, resp)
}

// 创建注册表
func (l *EnvHandle) CreateLabResource(ctx *gin.Context) {
	req := &environment.ResourceReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	err := l.envService.CreateResource(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "CreateLabResource err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx)
}

func (l *EnvHandle) GetLabMember(ctx *gin.Context) {
	req := &environment.LabMemberReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	if err := ctx.ShouldBindQuery(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.LabMemberList(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "GetLabMember err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) DelLabMember(ctx *gin.Context) {
	req := &environment.DelLabMemberReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	err := l.envService.DelLabMember(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "DelLabMember err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx)
}

func (l *EnvHandle) CreateInvite(ctx *gin.Context) {
	req := &environment.InviteReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	if err := ctx.ShouldBindJSON(req); err != nil && err.Error() != "EOF" {
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.CreateInvite(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "CreateInvite err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l EnvHandle) AcceptInvite(ctx *gin.Context) {
	req := &environment.AcceptInviteReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.AcceptInvite(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "AcceptInvite err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) UserInfo(ctx *gin.Context) {
	resp, err := l.envService.UserInfo(ctx)
	common.Reply(ctx, err, resp)
}

func (l *EnvHandle) PinLab(ctx *gin.Context) {
	req := &environment.PinLabReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithErr(err))
		return
	}

	common.Reply(ctx, l.envService.PinLab(ctx, req))
}

func (l *EnvHandle) Policy(ctx *gin.Context) {
	req := &environment.PolicyReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithErr(err))
		return
	}

	resp, err := l.envService.Policy(ctx, req)
	common.Reply(ctx, err, resp)
}

func (l *EnvHandle) PolicyResource(ctx *gin.Context) {
	resp, err := l.envService.PolicyResource(ctx)
	common.Reply(ctx, err, resp)
}

func (l *EnvHandle) CreateRole(ctx *gin.Context) {
	req := &environment.CreateRoleReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.CreateRole(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "CreateRole err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) RoleList(ctx *gin.Context) {
	req := &environment.RoleListReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.RoleList(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "RoleList err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) DelRole(ctx *gin.Context) {
	req := &environment.DelRoleReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithErr(err))
		return
	}

	err := l.envService.DelRole(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "DelRole err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx)
}

func (l *EnvHandle) ModifyRolePerm(ctx *gin.Context) {
	req := &environment.AddRolePermReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	err := l.envService.ModifyRolePerm(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "AddRolePerm err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.Reply(ctx, err)
}

func (l *EnvHandle) RolePermList(ctx *gin.Context) {
	req := &environment.RolePermListReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.RolePermList(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "RolePermList err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) DelRolePerm(ctx *gin.Context) {
	req := &environment.DelRolePermReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	err := l.envService.DelRolePerm(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "DelRolePerm err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx)
}

func (l *EnvHandle) ModifyUserRole(ctx *gin.Context) {
	req := &environment.AddUserRoleReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	resp, err := l.envService.ModifyUserRole(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "ModifyUserRole err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx, resp)
}

func (l *EnvHandle) DelUserRole(ctx *gin.Context) {
	req := &environment.DelUserRoleReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse body err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}

	err := l.envService.DelUserRole(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "DelUserRole err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}

	common.ReplyOk(ctx)
}
