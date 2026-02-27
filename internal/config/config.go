package config

import (
	"fmt"
	"os"

	"github.com/creasty/defaults"
)

type GlobalConfig struct {
	Database Database `mapstructure:",squash"`
	Redis    Redis    `mapstructure:",squash"`
	Server   Server   `mapstructure:",squash"`
	OAuth2   OAuth2   `mapstructure:",squash"`
	Auth     Auth     `mapstructure:",squash"`
	RPC      RPC      `mapstructure:",squash"`
	Log      Log      `mapstructure:",squash"`
	Trace    Trace    `mapstructure:",squash"`
	Job      Job      `mapstructure:",squash"`
	Sandbox  Sandbox  `mapstructure:",squash"`
}

var config = &GlobalConfig{}

func init() {
	if err := defaults.Set(config); err != nil {
		fmt.Printf("set default err: %+v", err)
		os.Exit(1)
	}
}

func Global() *GlobalConfig {
	return config
}
