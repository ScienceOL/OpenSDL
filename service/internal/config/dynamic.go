package config

import (
	// 外部依赖
	"fmt"
	"net/url"
)

type DynamicConfig struct {
	Test       string   `yaml:"test"`
	RetryCount int      `yaml:"retry_count"`
	Interval   int      `yaml:"interval"`
	Machine    Machine  `yaml:"machine"`
	Storage    Storage  `yaml:"storage"`
	Schedule   Schedule `yaml:"schedule"`
}

type Machine struct {
	ImageID      uint64  `yaml:"image_id"`
	SkuID        int64   `yaml:"sku_id"`
	ProjectID    uint64  `yaml:"project_id"`
	TurnoffAfter float32 `yaml:"turnoff_after"`
	DiskSize     uint    `yaml:"disk_size"`
	Cmd          string  `yaml:"cmd"`
}

type Storage struct {
	OssAddr            string `yaml:"addr"`
	OssAccessKeyID     string `yaml:"app_key"`
	OssAccessKeySecret string `yaml:"app_secret"`
	OssEndpoint        string `yaml:"endpoint"`
	OssBucketName      string `yaml:"bucket_name"`
	TokenTTL           int64  `yaml:"token_ttl"` // Token 过期时间（小时）
}

func (s Storage) Image(icon string) string {
	if icon == "" || s.OssBucketName == "" || s.OssEndpoint == "" {
		return ""
	}

	iu := url.URL{
		Scheme: "https",
		Host:   fmt.Sprintf("%s.%s", s.OssBucketName, s.OssEndpoint[len("https://"):]),
		Path:   fmt.Sprintf("media/device_icon/%s", icon),
	}

	return iu.String()
}

type Schedule struct {
	TranslateNodeParam bool `yaml:"translate_node_param"`
}
