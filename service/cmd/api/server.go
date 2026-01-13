package api

import (
	// 外部依赖
	cobra "github.com/spf13/cobra"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	db "github.com/scienceol/opensdl/service/pkg/middleware/db"
)

func NewWeb() *cobra.Command {
	webServer := &cobra.Command{
		Use:  "apiserver",
		Long: `api server start`,

		// stop printing usage when the command errors
		SilenceUsage: true,
		// PreRunE:      initWeb,
		// RunE:         newRouter,
		// PostRunE:     cleanWebResource,
	}

	return webServer
}

func initMigrate(cmd *cobra.Command, _ []string) error {
	config := config.Global()
	// 初始化数据库
	db.InitPostgres(cmd.Context(), &db.Config{
		Host:   config.Database.Host,
		Port:   config.Database.Port,
		User:   config.Database.User,
		PW:     config.Database.Password,
		DBName: config.Database.Name,
		LogConf: db.LogConf{
			Level: config.Log.LogLevel,
		},
	})

	return nil
}
