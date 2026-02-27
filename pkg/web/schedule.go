package web

import (
	"context"

	"github.com/gin-gonic/gin"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/web/views/health"
	"github.com/scienceol/osdl/pkg/web/views/schedule"
)

func NewSchedule(ctx context.Context, g *gin.Engine) context.CancelFunc {
	installMiddleware(g)
	return installScheduleURL(ctx, g)
}

func installScheduleURL(ctx context.Context, g *gin.Engine) context.CancelFunc {
	api := g.Group("/api")
	api.GET("/health", health.Health)
	api.GET("/health/live", health.Live)
	api.GET("/health/ready", health.Ready)
	handle := schedule.New(ctx)

	{
		v1 := api.Group("/v1/ws", auth.AuthLab())
		v1.GET("/schedule", handle.Connect)
	}

	return func() {
		handle.Close(ctx)
	}
}
