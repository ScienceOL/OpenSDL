package web

import (
	// 外部依赖
	"context"
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
