package api

import (
	"context"
	"crypto/tls"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"strconv"
	"time"

	_ "github.com/scienceol/osdl/docs" // swagger generated docs

	"github.com/gin-gonic/gin"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/core/notify/events"
	osdlgrpc "github.com/scienceol/osdl/pkg/grpc"
	"github.com/scienceol/osdl/pkg/middleware/db"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/middleware/redis"
	"github.com/scienceol/osdl/pkg/middleware/trace"
	migrate "github.com/scienceol/osdl/pkg/repo/migrate"
	"github.com/scienceol/osdl/pkg/utils"
	"github.com/scienceol/osdl/pkg/web"
	"github.com/spf13/cobra"
)

func NewWeb() *cobra.Command {
	return &cobra.Command{
		Use:          "apiserver",
		Long:         "Start the API server (HTTP + gRPC)",
		SilenceUsage: true,
		PreRunE:      initWeb,
		RunE:         newRouter,
		PostRunE:     cleanWebResource,
	}
}

func NewMigrate() *cobra.Command {
	return &cobra.Command{
		Use:          "migrate",
		Long:         "Run database migrations",
		SilenceUsage: true,
		PreRunE:      initMigrate,
		RunE: func(cmd *cobra.Command, _ []string) error {
			return migrate.Table(cmd.Root().Context())
		},
		PostRunE: func(cmd *cobra.Command, _ []string) error {
			db.ClosePostgres(cmd.Context())
			return nil
		},
	}
}

func initMigrate(cmd *cobra.Command, _ []string) error {
	conf := config.Global()
	db.InitPostgres(cmd.Context(), &db.Config{
		Host: conf.Database.Host, Port: conf.Database.Port,
		User: conf.Database.User, PW: conf.Database.Password,
		DBName: conf.Database.Name, LogConf: db.LogConf{Level: conf.Log.LogLevel},
	})
	return nil
}

func initWeb(cmd *cobra.Command, _ []string) error {
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
	// Generate Swagger documentation at startup
	swagCmd := exec.Command("swag", "init", "-g", "main.go")
	swagCmd.Dir = "."
	output, err := swagCmd.CombinedOutput()
	if err != nil {
		logger.Warnf(cmd.Context(), "Could not generate Swagger docs: %v. Output: %s", err, string(output))
	} else {
		logger.Infof(cmd.Context(), "Swagger documentation generated successfully")
	}

	router := gin.Default()
	web.NewRouter(cmd.Root().Context(), router)
	conf := config.Global()
	port := conf.Server.Port
	addr := ":" + strconv.Itoa(port)

	httpServer := http.Server{
		Addr:              addr,
		Handler:           router,
		ReadHeaderTimeout: 30 * time.Second,
		IdleTimeout:       30 * time.Second,
		TLSNextProto:      make(map[string]func(*http.Server, *tls.Conn, http.Handler)),
	}

	fmt.Printf("API Server starting on http://0.0.0.0:%d\n", port)

	utils.SafelyGo(func() {
		if err := httpServer.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			logger.Errorf(cmd.Context(), "start server err: %v\n", err)
		}
	}, func(err error) {
		logger.Errorf(cmd.Context(), "run http server err: %+v", err)
		os.Exit(1)
	})

	// Start gRPC server
	grpcPort := conf.Server.GrpcPort
	grpcServer, err := osdlgrpc.NewServer(cmd.Root().Context(), grpcPort)
	if err != nil {
		logger.Errorf(cmd.Context(), "start gRPC server err: %+v", err)
	} else {
		fmt.Printf("gRPC Server starting on port %d\n", grpcPort)
	}

	fmt.Printf("Server started. Press Ctrl+C to shutdown.\n")
	<-cmd.Context().Done()

	// Graceful shutdown
	if grpcServer != nil {
		grpcServer.GracefulStop()
	}
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
	if err := httpServer.Shutdown(ctx); err != nil {
		fmt.Printf("shut down server err: %+v", err)
	}
	return nil
}

func cleanWebResource(cmd *cobra.Command, _ []string) error {
	events.NewEvents().Close(cmd.Context())
	redis.CloseRedis(cmd.Context())
	db.ClosePostgres(cmd.Context())
	trace.CloseTrace()
	return nil
}
