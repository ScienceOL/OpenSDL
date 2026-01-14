package repo

import (
	// 外部依赖
	"context"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
)

type ResourcePerm struct {
	Permissions map[string][]common.Perm `json:"permissions"`
}

type RoleResReq struct {
	Role     common.Role `json:"role"`
	Resource string      `json:"resource"`
}

type RoleResResp struct {
	AvailableActions []common.Perm `json:"available_actions"`
	Resource         string        `json:"resource"`
}

type UserPermRes struct {
	LabID  int64
	UserID string
	Roles  []string
}

type Policy interface {
	GetRolePerms(ctx context.Context, req *UserPermRes) (*ResourcePerm, error)
	GetPermByRole(ctx context.Context, req *RoleResReq) (*RoleResResp, error)
}
