package mcp

import (
	// 外部依赖
	"context"

	// 内部引用
	action "github.com/scienceol/opensdl/service/pkg/core/schedule/engine/action"
)

type Service interface {
	RunAction(ctx context.Context, req *action.RunActionReq) (*action.RunActionResp, error)
	QueryTaskStatus(ctx context.Context, req *TaskStatusReq) (*TaskStatusResp, error)
}
