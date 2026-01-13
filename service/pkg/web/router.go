package web

import (
	// 外部依赖
	"context"

	gin "github.com/gin-gonic/gin"
)

func NewRouter(ctx context.Context, g *gin.Engine) {
	installMiddleware(g)
	InstallURL(ctx, g)
}