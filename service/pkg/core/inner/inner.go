package inner

import "context"

type Service interface {
	GetUserCustomPolicy(ctx context.Context, req *CustomPolicyReq) (*CustomPolicyResp, error)
	GetResources(ctx context.Context) (*ResouceResp, error)
}
