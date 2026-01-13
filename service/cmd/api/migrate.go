package api

import (
	// 外部依赖
	cobra "github.com/spf13/cobra"

	// 内部引用
	db "github.com/scienceol/opensdl/service/pkg/middleware/db"
	migrate "github.com/scienceol/opensdl/service/pkg/model/migrate"
)

func NewMigrate() *cobra.Command {
	return &cobra.Command{
		Use:          "migrate",
		Long:         `api server db migrate`,
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
