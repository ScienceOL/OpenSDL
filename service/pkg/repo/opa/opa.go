package opa

import (
	// 外部依赖
	"context"
	"net/http"

	resty "github.com/go-resty/resty/v2"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
)

type Res[T any] struct {
	Result common.RespT[T]
}

type opaClient struct {
	opaClient *resty.Client
}

func NewOpaClient() repo.Policy {
	conf := config.Global().RPC.Opa
	return &opaClient{
		opaClient: resty.New().
			EnableTrace().
			SetBaseURL(conf.Addr),
	}
}

// 获取角色的所有资源权限
func (o *opaClient) GetRolePerms(ctx context.Context, req *repo.UserPermRes) (*repo.ResourcePerm, error) {
	resData := &Res[*repo.ResourcePerm]{}
	resp, err := o.opaClient.R().SetContext(ctx).
		SetBody(map[string]any{
			"input": map[string]any{
				"lab_id":  req.LabID,
				"user_id": req.UserID,
				"roles":   req.Roles,
			},
		}).
		SetResult(resData).
		Post("/v1/data/policies/authz/rbac_role/get_role_perms")
	if err != nil {
		logger.Errorf(ctx, "GetRolePerms err: %+v role: %+v", err, *req)
		return nil, code.RPCHttpErr.WithMsg(err.Error())
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "GetRolePerms http code: %d", resp.StatusCode())
		return nil, code.RPCHttpCodeErr
	}

	if resData.Result.Data == nil || resData.Result.Code != 0 {
		logger.Errorf(ctx, "GetRolePerms res data err")
		return nil, code.RPCHttpCodeRespErr
	}

	return resData.Result.Data, nil
}

// 根据角色资源获取权限
func (o *opaClient) GetPermByRole(ctx context.Context, req *repo.RoleResReq) (*repo.RoleResResp, error) {
	resData := &Res[*repo.RoleResResp]{}
	resp, err := o.opaClient.R().SetContext(ctx).
		SetBody(map[string]any{
			"input": req,
		}).
		SetResult(resData).
		Post("/v1/data/policies/authz/rbac/get_permission")
	if err != nil {
		logger.Errorf(ctx, "GetPermByRole err: %+v role: %+v", err, *req)
		return nil, code.RPCHttpErr.WithMsg(err.Error())
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "GetPermByRole http code: %d", resp.StatusCode())
		return nil, code.RPCHttpCodeErr
	}

	if resData.Result.Data == nil || resData.Result.Code != 0 {
		logger.Errorf(ctx, "GetPermByRole res data err")
		return nil, code.RPCHttpCodeRespErr
	}

	return resData.Result.Data, nil
}
