package schedule

import (
	// 外部依赖
	"context"

	gin "github.com/gin-gonic/gin"

	// 内部引用
	schedule "github.com/scienceol/opensdl/service/pkg/core/schedule"
	control "github.com/scienceol/opensdl/service/pkg/core/schedule/control"
)

type Handle struct {
	ctrl schedule.Control
}

func New(ctx context.Context) *Handle {
	return &Handle{
		ctrl: control.NewControl(ctx),
	}
}

func (m *Handle) Connect(ctx *gin.Context) {
	m.ctrl.Connect(ctx)
}

func (m *Handle) Close(ctx context.Context) {
	m.ctrl.Close(ctx)
}
