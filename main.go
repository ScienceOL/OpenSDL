package main

import (
	"log"
	"os"

	"github.com/joho/godotenv"
	"github.com/scienceol/osdl/cmd/api"
	"github.com/scienceol/osdl/cmd/schedule"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/utils"
	"github.com/spf13/cobra"
	"github.com/spf13/viper"
)

func main() {
	rootCtx := utils.SetupSignalContext()
	root := &cobra.Command{
		SilenceUsage: true,
		Short:        "osdl",
		Long:         "OSDL - Open Science Device Lab communication infrastructure",
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
	if err := godotenv.Load(); err != nil {
		log.Println("No .env file found - using environment variables")
	}

	v := viper.NewWithOptions(viper.ExperimentalBindStruct())
	v.AutomaticEnv()

	conf := config.Global()
	if err := v.Unmarshal(conf); err != nil {
		log.Fatal(err)
	}

	logger.Init(&logger.LogConfig{
		Path:     conf.Log.LogPath,
		LogLevel: conf.Log.LogLevel,
		ServiceEnv: logger.ServiceEnv{
			Platform: conf.Server.Platform,
			Service:  conf.Server.Service,
			Env:      conf.Server.Env,
		},
	})

	return nil
}

func cleanGlobalResource(_ *cobra.Command, _ []string) error {
	logger.Close()
	return nil
}
