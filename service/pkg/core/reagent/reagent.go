package reagent

import (
	// 外部依赖
	"context"

	// 内部引用
	common "github.com/scienceol/opensdl/service/pkg/common"
)

// Service 定义试剂相关业务接口
// 目前仅包含插入接口，可按需扩展
//
// 所有方法均接受 context.Context，web 层可直接传入 *gin.Context
// 以便在实现内部获取用户信息、日志、DB会话等。
type Service interface {
	// Insert 新增试剂
	Insert(ctx context.Context, req *InsertReq) (*InsertResp, error)
	// Query 查询试剂
	Query(ctx context.Context, req *QueryReq) (*common.PageResp[[]*ReagentResponse], error)
	// Delete 删除试剂
	Delete(ctx context.Context, req *DeleteReq) error
	// Update 批量更新试剂
	Update(ctx context.Context, req *UpdateReq) error
	// QueryCAS 通过 CAS 查询基础信息
	QueryCAS(ctx context.Context, req *CasReq) (*CasResp, error)
}
