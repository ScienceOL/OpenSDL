package foo

import (
	// 外部依赖
	"net/http"

	gin "github.com/gin-gonic/gin"

	// 内部引用
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
)

func HandleHelloWorld(ctx *gin.Context) {
	// 使用GetCurrentUser获取当前用户
	user := auth.GetCurrentUser(ctx)
	if user == nil {
		// 由于已经使用了RequireAuth中间件，这种情况正常不会发生
		ctx.JSON(http.StatusForbidden, gin.H{
			"error": "Forbidden: Not authenticated",
		})
	}

	logger.Infof(ctx.Request.Context(), "User %v is authenticated", user)

	// 用户已登录，返回欢迎信息
	ctx.JSON(http.StatusOK, gin.H{
		"message": "Hello World",
		"user":    user,
	})
}
