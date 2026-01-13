package schedule

import (
	// 外部依赖
	"fmt"
	"context"
	"net/http"
	"crypto/tls"
	"os"
	"strconv"
	"time"

	gin "github.com/gin-gonic/gin"
	cobra "github.com/spf13/cobra"
	yaml "gopkg.in/yaml.v2"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	nacos "github.com/scienceol/opensdl/service/pkg/middleware/nacos"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	db "github.com/scienceol/opensdl/service/pkg/middleware/db"
	trace "github.com/scienceol/opensdl/service/pkg/middleware/trace"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
	redis "github.com/scienceol/opensdl/service/pkg/middleware/redis"
	web "github.com/scienceol/opensdl/service/pkg/web"
	events "github.com/scienceol/opensdl/service/pkg/core/notify/events"
)

func New() *cobra.Command {
	return &cobra.Command{
		Use:                "schedule",
		Long:               `api server workflow schedule`,
		SilenceUsage:       true,
		PreRunE:            initSchedule,
		RunE:               newRouter,
		PostRunE:           cleanSchedule,
	}
}

func initSchedule(cmd *cobra.Command, _ []string) error {
	conf := config.Global()
	// 初始化 nacos , 注意初始化时序，请勿在动态配置未初始化时候使用配置
	nacos.MustInit(cmd.Context(), &nacos.Conf{
		Endpoint:    conf.Nacos.Endpoint,
		User:        conf.Nacos.User,
		Password:    conf.Nacos.Password,
		Port:        conf.Nacos.Port,
		DataID:      conf.Nacos.DataID,
		Group:       conf.Nacos.Group,
		NeedWatch:   conf.Nacos.NeedWatch,
		NamespaceID: conf.Nacos.NamespaceID,
		AccessKey:   conf.Nacos.AccessKey,
		SecretKey:   conf.Nacos.SecretKey,
		RegionID:    conf.Nacos.RegionID,
	},
		func(content []byte) error {
			d := &config.DynamicConfig{}
			if err := yaml.Unmarshal(content, d); err != nil {
				logger.Errorf(cmd.Context(),
					"Unmarshal nacos config fail dataID: %s, Group: %s, err: %+v",
					conf.Nacos.DataID, conf.Nacos.Group, err)
			} else {
				conf.SetDynamic(d)
			}
			return nil
		})

	// 初始化 trace
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

	// 初始化数据库
	db.InitPostgres(cmd.Context(), &db.Config{
		Host:   conf.Database.Host,
		Port:   conf.Database.Port,
		User:   conf.Database.User,
		PW:     conf.Database.Password,
		DBName: conf.Database.Name,
		LogConf: db.LogConf{
			Level: conf.Log.LogLevel,
		},
	})

	// 初始化 redis
	redis.InitRedis(cmd.Context(), &redis.Redis{
		Host:     conf.Redis.Host,
		Port:     conf.Redis.Port,
		Password: conf.Redis.Password,
		DB:       conf.Redis.DB,
	})

	return nil
}

func newRouter(cmd *cobra.Command, _ []string) error {
	router := gin.Default()

	cancel := web.NewSchedule(cmd.Root().Context(), router)
	port := config.Global().Server.SchedulePort
	addr := ":" + strconv.Itoa(port)

	httpServer := http.Server{
		Addr:              ":" + strconv.Itoa(port),
		Handler:           router,
		ReadHeaderTimeout: 30 * time.Second,
		WriteTimeout:      30 * time.Second,
		TLSNextProto:      make(map[string]func(*http.Server, *tls.Conn, http.Handler)),
	}

	// 添加启动成功的日志输出
	fmt.Printf("🚀 Server starting on http://localhost:%d\n", port)
	fmt.Printf("📡 API Server is running at: http://0.0.0.0:%d\n", port)
	fmt.Printf("🔧 Server configuration: %+v\n", addr)

	// 异步监听端口
	utils.SafelyGo(func() {
		if err := httpServer.ListenAndServe(); err != nil {
			if err != http.ErrServerClosed {
				logger.Errorf(cmd.Context(), "start server err: %v\n", err)
			}
		}
	}, func(err error) {
		logger.Errorf(cmd.Context(), "run http server err: %+v", err)
		os.Exit(1)
	})

	// 服务启动成功提示
	fmt.Printf("✅ Server successfully started on port %d\n", port)
	fmt.Println("Press Ctrl+C to gracefully shutdown the server...")

	// 阻塞等待收到中断信号
	<-cmd.Context().Done()

	cancel()
	// 平滑超时退出
	ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
	defer cancel()
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