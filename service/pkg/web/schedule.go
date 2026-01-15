package web

import (
	// 外部依赖
	"context"

	gin "github.com/gin-gonic/gin"
	_ "github.com/scienceol/opensdl/service/docs" // 导入自动生成的 docs 包
	swaggerfiles "github.com/swaggo/files"
	ginSwagger "github.com/swaggo/gin-swagger"

	// 内部引用
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	views "github.com/scienceol/opensdl/service/pkg/web/views"
	schedule "github.com/scienceol/opensdl/service/pkg/web/views/schedule"
)

func NewSchedule(ctx context.Context, g *gin.Engine) context.CancelFunc {
	installMiddleware(g)
	return InstallScheduleURL(ctx, g)
}

func InstallScheduleURL(ctx context.Context, g *gin.Engine) context.CancelFunc {
	api := g.Group("/api")
	api.GET("/health", views.Health)
	api.GET("/swagger/*any", ginSwagger.WrapHandler(swaggerfiles.Handler))
	handle := schedule.New(ctx)

	{
		v1 := api.Group("/v1/ws", auth.AuthLab())
		v1.GET("/schedule", handle.Connect)
	}

	return func() {
		handle.Close(ctx)
	}
}
