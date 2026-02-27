package web

import (
	"context"
	"fmt"

	"github.com/gin-contrib/cors"
	"github.com/gin-gonic/gin"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/web/views/health"
	"github.com/scienceol/osdl/pkg/web/views/login"
	materialView "github.com/scienceol/osdl/pkg/web/views/material"
	"github.com/scienceol/osdl/pkg/web/views/sse"
	"go.opentelemetry.io/contrib/instrumentation/github.com/gin-gonic/gin/otelgin"
)

func NewRouter(ctx context.Context, g *gin.Engine) {
	installMiddleware(g)
	installURL(ctx, g)
}

func installMiddleware(g *gin.Engine) {
	g.ContextWithFallback = true
	server := config.Global().Server
	g.Use(cors.Default())
	g.Use(otelgin.Middleware(fmt.Sprintf("%s-%s", server.Platform, server.Service)))
	g.Use(logger.LogWithWriter())
}

func installURL(ctx context.Context, g *gin.Engine) {
	api := g.Group("/api")
	api.GET("/health", health.Health)
	api.GET("/health/live", health.Live)
	api.GET("/health/ready", health.Ready)

	// Auth routes
	{
		l := login.NewLogin()
		authGroup := api.Group("/auth")
		authGroup.GET("/login", l.Login)
		authGroup.GET("/callback/casdoor", l.Callback)
		authGroup.POST("/refresh", l.Refresh)
	}

	// Material handler
	mHandle := materialView.NewMaterialHandle(ctx)

	// Protected routes
	{
		v1 := api.Group("/v1")

		// WebSocket routes
		wsRouter := v1.Group("/ws", auth.AuthWeb())
		{
			wsRouter.GET("/material/:lab_uuid", mHandle.LabMaterial)
		}

		labRouter := v1.Group("/lab", auth.AuthWeb())

		// Material CRUD routes
		{
			materialRouter := labRouter.Group("/material")
			materialRouter.POST("/create", mHandle.CreateLabMaterial)
			materialRouter.POST("/save", mHandle.SaveMaterial)
			materialRouter.GET("/query", mHandle.QueryMaterial)
			materialRouter.POST("/query/uuid", mHandle.QueryMaterialByUUID)
			materialRouter.PUT("/update", mHandle.BatchUpdateMaterial)
			materialRouter.POST("/edge", mHandle.CreateMaterialEdge)
			materialRouter.GET("/download/:lab_uuid", mHandle.DownloadMaterial)
			materialRouter.GET("/template/:lab_uuid", mHandle.Template)
			materialRouter.GET("/resource/template", mHandle.GetResourceNodeTemplate)
			materialRouter.GET("/resource/list", mHandle.ResourceList)
			materialRouter.GET("/actions", mHandle.Actions)
			materialRouter.POST("/machine/start", mHandle.StartMachine)
			materialRouter.POST("/machine/stop", mHandle.StopMachine)
			materialRouter.DELETE("/machine", mHandle.DeleteMachine)
			materialRouter.GET("/machine/status", mHandle.MachineStatus)
		}

		// Edge routes (lab device reporting)
		{
			edgeRouter := v1.Group("/edge", auth.AuthLab())
			edgeRouter.POST("/material/create", mHandle.EdgeCreateMaterial)
			edgeRouter.POST("/material/upsert", mHandle.EdgeUpsertMaterial)
			edgeRouter.POST("/material/edge", mHandle.EdgeCreateEdge)
			edgeRouter.GET("/material/download", mHandle.EdgeDownloadMaterial)
		}

		// SSE notifications
		{
			streamRouter := labRouter.Group("/notify")
			streamRouter.GET("/sse", sse.Notify)
		}
	}
}
