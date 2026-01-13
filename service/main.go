package main

import (
	// 外部依赖
	"log"
	"os"

	godotenv "github.com/joho/godotenv"
	cobra "github.com/spf13/cobra"
	viper "github.com/spf13/viper"

	// 内部引用
	api "github.com/scienceol/opensdl/service/cmd/api"
	schedule "github.com/scienceol/opensdl/service/cmd/schedule"
	config "github.com/scienceol/opensdl/service/internal/config"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

func main() {
	rootCtx := utils.SetupSignalContext()
	root := &cobra.Command{
		SilenceUsage:      true,
		Short:             "OpenSDL",
		Long:              "OpenSDL 开放自驱实验室后端服务",
		PersistentPreRunE: initGlobalResource,
		Run: func(cmd *cobra.Command, _ []string) {
			_ = cmd.Help()
		},
		PersistentPostRunE: cleanGlobalResource,
	}
	root.SetContext(rootCtx)
	root.AddCommand(api.NewWeb())
	root.AddCommand(api.NewMigrate())
	root.AddCommand(schedule.New())

	if err := root.Execute(); err != nil {
		os.Exit(1)
	}
}

func initGlobalResource(_ *cobra.Command, _ []string) error {
	// 初始化全局环境变量
	if err := godotenv.Load(); err != nil {
		log.Println("No .env file found - using environment variables")
	}

	v := viper.NewWithOptions(viper.ExperimentalBindStruct())
	v.AutomaticEnv()

	config := config.Global()
	if err := v.Unmarshal(config); err != nil {
		log.Fatal(err)
	}

	// 日志初始化
	logger.Init(&logger.LogConfig{
		Path:     config.Log.LogPath,
		LogLevel: config.Log.LogLevel,
		ServiceEnv: logger.ServiceEnv{
			Platform: config.Server.Platform,
			Service:  config.Server.Service,
			Env:      config.Server.Env,
		},
	})

	return nil
}

func cleanGlobalResource(_ *cobra.Command, _ []string) error {
	// 服务退出清理资源
	logger.Close()
	return nil
}
