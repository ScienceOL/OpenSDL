package health

import (
	"net/http"

	"github.com/gin-gonic/gin"
	"github.com/scienceol/osdl/pkg/middleware/db"
	"github.com/scienceol/osdl/pkg/middleware/redis"
)

// Health is a simple health check (backward compatible).
func Health(g *gin.Context) {
	g.JSON(http.StatusOK, gin.H{"status": "ok"})
}

// Live is a lightweight liveness probe — the process is alive.
func Live(g *gin.Context) {
	g.JSON(http.StatusOK, gin.H{"status": "ok"})
}

// Ready is a readiness probe — verifies all downstream dependencies.
func Ready(g *gin.Context) {
	checks := gin.H{}
	healthy := true

	// PostgreSQL
	if ds := db.DB(); ds != nil {
		sqlDB, err := ds.DBIns().DB()
		if err != nil || sqlDB.PingContext(g.Request.Context()) != nil {
			checks["postgres"] = "unhealthy"
			healthy = false
		} else {
			checks["postgres"] = "ok"
		}
	} else {
		checks["postgres"] = "not_initialized"
		healthy = false
	}

	// Redis
	if rc := redis.GetClient(); rc != nil {
		if err := rc.Ping(g.Request.Context()).Err(); err != nil {
			checks["redis"] = "unhealthy"
			healthy = false
		} else {
			checks["redis"] = "ok"
		}
	} else {
		checks["redis"] = "not_initialized"
		healthy = false
	}

	status := http.StatusOK
	msg := "ready"
	if !healthy {
		status = http.StatusServiceUnavailable
		msg = "not_ready"
	}

	g.JSON(status, gin.H{
		"status": msg,
		"checks": checks,
	})
}
