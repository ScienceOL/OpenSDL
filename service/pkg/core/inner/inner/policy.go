package inner

import (
	// 外部依赖
	"context"
	"fmt"

	inner "github.com/scienceol/opensdl/service/pkg/core/inner"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	eStore "github.com/scienceol/opensdl/service/pkg/repo/environment"
	model "github.com/scienceol/opensdl/service/pkg/model"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type InnerImpl struct {
	envStore repo.LaboratoryRepo
}

func NewInner() inner.Service {
	return &InnerImpl{
		envStore: eStore.New(),
	}
}

func (i *InnerImpl) GetUserCustomPolicy(ctx context.Context, req *inner.CustomPolicyReq) (*inner.CustomPolicyResp, error) {
	resp, err := i.getCustomPolicyFromDB(ctx, req)
	if err != nil {
		return nil, err
	}
	return resp, nil
}

func (i *InnerImpl) getCustomPolicyFromDB(ctx context.Context, req *inner.CustomPolicyReq) (*inner.CustomPolicyResp, error) {
	res := &inner.CustomPolicyResp{RolePerm: map[string][]string{}}

	// 获取用户拥有角色
	userRoles := make([]*model.UserRole, 0, 1)
	if err := i.envStore.FindDatas(ctx, &userRoles, map[string]any{
		"lab_id":  req.LabID,
		"user_id": req.UserID,
	}); err != nil {
		return nil, err
	}

	if len(userRoles) == 0 {
		return res, nil
	}

	customRoleIDs := utils.FilterSlice(userRoles, func(u *model.UserRole) (int64, bool) {
		return u.CustomRoleID, true
	})

	// 获取所有自定义角色
	customRoles := make([]*model.CustomRole, 0, 1)
	if err := i.envStore.FindDatas(ctx, &customRoles, map[string]any{
		"id": customRoleIDs,
	}); err != nil {
		return nil, err
	}

	if len(customRoles) == 0 {
		return res, nil
	}

	// 获取所有角色对应的权限
	customRolePerms := make([]*model.CustomRolePerm, 0, 1)
	if err := i.envStore.FindDatas(ctx, &customRolePerms, map[string]any{
		"custom_role_id": customRoleIDs,
	}); err != nil {
		return nil, err
	}

	poilcyResourceIDs := utils.FilterUniqSlice(customRolePerms, func(p *model.CustomRolePerm) (int64, bool) {
		return p.PolicyResourceID, true
	})

	if len(poilcyResourceIDs) == 0 {
		return res, nil
	}

	// 获取所有的固定资源
	policyResources := make([]*model.PolicyResource, 0, 1)
	if err := i.envStore.FindDatas(ctx, &policyResources, map[string]any{
		"id": poilcyResourceIDs,
	}); err != nil {
		return nil, err
	}

	policyResourceMap := utils.Slice2Map(policyResources, func(p *model.PolicyResource) (int64, *model.PolicyResource) {
		return p.ID, p
	})

	roleMap := utils.Slice2Map(customRoles, func(c *model.CustomRole) (int64, *model.CustomRole) {
		return c.ID, c
	})

	roleGroup := utils.Slice2MapSlice(customRolePerms, func(p *model.CustomRolePerm) (int64, *model.CustomRolePerm, bool) {
		return p.CustomRoleID, p, true
	})

	for roleID, rolePerms := range roleGroup {
		role, ok := roleMap[roleID]
		if !ok {
			continue
		}
		res.RolePerm[role.RoleName] = utils.FilterSlice(rolePerms, func(rp *model.CustomRolePerm) (string, bool) {
			resource, ok := policyResourceMap[rp.PolicyResourceID]
			return fmt.Sprintf("%s:%s", resource.Name, rp.Perm), ok
		})
	}

	return res, nil
}

func (i *InnerImpl) GetResources(ctx context.Context) (*inner.ResouceResp, error) {
	res := make([]*model.PolicyResource, 0, 10)
	if err := i.envStore.FindDatas(ctx, &res, nil); err != nil {
		return nil, err
	}
	return &inner.ResouceResp{
		Resource: utils.Slice2Map(res, func(r *model.PolicyResource) (string, *inner.ResourceDetail) {
			return r.Name, &inner.ResourceDetail{
				Description: r.Description,
			}
		}),
	}, nil
}
