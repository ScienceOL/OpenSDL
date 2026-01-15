package auth

import (
	// 外部依赖
	oauth2 "golang.org/x/oauth2"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
)

type Config struct {
	ClientID     string
	ClientSecret string
	Scopes       []string
	TokenURL     string
	AuthURL      string
	RedirectURL  string
	UserInfoURL  string
}

var (
	oauthConfig *oauth2.Config

	USERKEY = "AUTH_USER_KEY"
	// LABKEY  = "AUTH_LAB_KEY"
)

// GetOAuthConfig 获取OAuth2配置
func GetOAuthConfig() *oauth2.Config {
	if oauthConfig == nil {
		authConf := config.Global().OAuth2
		oauthConfig = &oauth2.Config{
			ClientID:     authConf.ClientID,
			ClientSecret: authConf.ClientSecret,
			Scopes:       authConf.Scopes,
			Endpoint: oauth2.Endpoint{
				TokenURL: authConf.TokenURL,
				AuthURL:  authConf.AuthURL,
			},
			RedirectURL: authConf.RedirectURL,
		}
	}

	return oauthConfig
}
