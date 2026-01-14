package environment

import (
	// 外部依赖
	"context"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
)

type EnvService interface {
	CreateLaboratoryEnv(ctx context.Context, req *LaboratoryEnvReq) (*LaboratoryEnvResp, error)
	UpdateLaboratoryEnv(ctx context.Context, req *UpdateEnvReq) (*LaboratoryResp, error)
	DelLab(ctx context.Context, req *DelLabReq) error
	LabInfo(ctx context.Context, req *LabInfoReq) (*LabInfoResp, error)
	CreateResource(ctx context.Context, req *ResourceReq) error
	LabList(ctx context.Context, req *common.PageReq) (*common.PageMoreResp[[]*LaboratoryResp], error)
	LabMemberList(ctx context.Context, req *LabMemberReq) (*common.PageResp[[]*LabMemberResp], error)
	DelLabMember(ctx context.Context, req *DelLabMemberReq) error
	CreateInvite(ctx context.Context, req *InviteReq) (*InviteResp, error)
	AcceptInvite(ctx context.Context, req *AcceptInviteReq) (*AcceptInviteResp, error)
	UserInfo(ctx context.Context) (*model.UserData, error)
	PinLab(ctx context.Context, req *PinLabReq) error
	Policy(ctx context.Context, req *PolicyReq) (*repo.ResourcePerm, error)

	CreateRole(ctx context.Context, req *CreateRoleReq) (*CreateRoleResp, error)
	RoleList(ctx context.Context, req *RoleListReq) (*RoleListResp, error)
	DelRole(ctx context.Context, req *DelRoleReq) error
	ModifyRolePerm(ctx context.Context, req *AddRolePermReq) error
	RolePermList(ctx context.Context, req *RolePermListReq) (*RolePermListResp, error)
	DelRolePerm(ctx context.Context, req *DelRolePermReq) error
	ModifyUserRole(ctx context.Context, req *AddUserRoleReq) (*AddUserRoleResp, error)
	DelUserRole(ctx context.Context, req *DelUserRoleReq) error
	PolicyResource(ctx context.Context) (*ResourceResp, error)

	// 项目相关
	CreateProject(ctx context.Context, req *CreateProjectReq) (*CreateProjectResp, error)
	ModifyProject(ctx context.Context, req *ModifyProjectReq) (*ModifyProjectResp, error)
	AddProjectUser(ctx context.Context, req *AddUserReq) (*AddUserResp, error)
	DelProjectUser(ctx context.Context, req *DelUserReq) error
	ProjectList(ctx context.Context, req *ProjectListReq) (*ProjectListResp, error)
}
