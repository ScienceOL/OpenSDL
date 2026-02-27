package auth

import (
	"sync"

	"github.com/scienceol/osdl/internal/config"
	"golang.org/x/oauth2"
)

var (
	oauthConfig *oauth2.Config
	oauthOnce   sync.Once
	USERKEY     = "AUTH_USER_KEY"
	LANGUAGE    = "Content-Language"
)

func GetOAuthConfig() *oauth2.Config {
	oauthOnce.Do(func() {
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
	})
	return oauthConfig
}
