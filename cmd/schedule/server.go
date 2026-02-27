package schedule

import (
	"context"
	"crypto/tls"
	"fmt"
	"net/http"
	"os"
	"strconv"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/core/notify/events"
	"github.com/scienceol/osdl/pkg/middleware/db"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/middleware/redis"
	"github.com/scienceol/osdl/pkg/middleware/trace"
	"github.com/scienceol/osdl/pkg/utils"
	"github.com/scienceol/osdl/pkg/web"
	"github.com/spf13/cobra"
)

func New() *cobra.Command {
	return &cobra.Command{
		Use:          "schedule",
		Long:         "Start the Schedule server (WebSocket + Redis consumer)",
		SilenceUsage: true,
		PreRunE:      initSchedule,
		RunE:         newRouter,
		PostRunE:     cleanSchedule,
	}
}

func initSchedule(cmd *cobra.Command, _ []string) error {
	conf := config.Global()
	trace.InitTrace(cmd.Context(), &trace.InitConfig{
		ServiceName:     fmt.Sprintf("%s-%s", conf.Server.Service, conf.Server.Platform),
		Version:         conf.Trace.Version,
		TraceEndpoint:   conf.Trace.TraceEndpoint,
		MetricEndpoint:  conf.Trace.MetricEndpoint,
		TraceProject:    conf.Trace.TraceProject,
		TraceInstanceID: conf.Trace.TraceInstanceID,
		TraceAK:         conf.Trace.TraceAK,
		TraceSK:         conf.Trace.TraceSK,
	})
	db.InitPostgres(cmd.Context(), &db.Config{
		Host: conf.Database.Host, Port: conf.Database.Port,
		User: conf.Database.User, PW: conf.Database.Password,
		DBName: conf.Database.Name, LogConf: db.LogConf{Level: conf.Log.LogLevel},
	})
	redis.InitRedis(cmd.Context(), &redis.Redis{
		Host: conf.Redis.Host, Port: conf.Redis.Port,
		Password: conf.Redis.Password, DB: conf.Redis.DB,
	})
	return nil
}

func newRouter(cmd *cobra.Command, _ []string) error {
	router := gin.Default()
	cancel := web.NewSchedule(cmd.Root().Context(), router)
	port := config.Global().Server.SchedulePort
	addr := ":" + strconv.Itoa(port)

	httpServer := http.Server{
		Addr:              addr,
		Handler:           router,
		ReadHeaderTimeout: 30 * time.Second,
		IdleTimeout:       120 * time.Second,
		TLSNextProto:      make(map[string]func(*http.Server, *tls.Conn, http.Handler)),
	}

	fmt.Printf("Schedule Server starting on http://0.0.0.0:%d\n", port)

	utils.SafelyGo(func() {
		if err := httpServer.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			logger.Errorf(cmd.Context(), "start server err: %v\n", err)
		}
	}, func(err error) {
		logger.Errorf(cmd.Context(), "run http server err: %+v", err)
		os.Exit(1)
	})

	fmt.Printf("Schedule Server started on port %d. Press Ctrl+C to shutdown.\n", port)
	<-cmd.Context().Done()

	cancel()
	ctx, cancelTimeout := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancelTimeout()
	if err := httpServer.Shutdown(ctx); err != nil {
		fmt.Printf("shut down server err: %+v", err)
	}
	return nil
}

func cleanSchedule(cmd *cobra.Command, _ []string) error {
	events.NewEvents().Close(cmd.Context())
	redis.CloseRedis(cmd.Context())
	db.ClosePostgres(cmd.Context())
	trace.CloseTrace()
	return nil
}
