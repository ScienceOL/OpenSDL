package casdoor

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"os"
	"time"

	r "github.com/redis/go-redis/v9"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/core/login"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/middleware/redis"
	"github.com/scienceol/osdl/pkg/repo/model"
	"golang.org/x/oauth2"
)

type oauthState struct {
	Timestamp           int64  `json:"timestamp"`
	FrontendCallbackURL string `json:"frontend_callback_url,omitempty"`
}

type casdoorLogin struct {
	*r.Client
	oauthConfig *oauth2.Config
}

func NewCasDoorLogin() login.Service {
	return &casdoorLogin{
		Client:      redis.GetClient(),
		oauthConfig: auth.GetOAuthConfig(),
	}
}

func (c *casdoorLogin) Login(ctx context.Context, req *login.LoginReq) (*login.Resp, error) {
	stateObj := oauthState{
		Timestamp:           time.Now().UnixNano(),
		FrontendCallbackURL: req.FrontendCallbackURL,
	}
	stateJSON, err := json.Marshal(stateObj)
	if err != nil {
		logger.Errorf(ctx, "Failed to marshal state: %v", err)
		return nil, code.LoginSetStateErr
	}
	state := base64.URLEncoding.EncodeToString(stateJSON)
	stateKey := fmt.Sprintf("oauth_state:%s", state)
	if err := c.Set(ctx, stateKey, "valid", 5*time.Minute).Err(); err != nil {
		logger.Errorf(ctx, "Failed to save state to Redis: %v", err)
		return nil, code.LoginSetStateErr
	}
	authURL := c.oauthConfig.AuthCodeURL(state, oauth2.AccessTypeOffline)
	return &login.Resp{RedirectURL: authURL}, nil
}

func (c *casdoorLogin) Refresh(ctx context.Context, req *login.RefreshTokenReq) (*login.RefreshTokenResp, error) {
	expiredToken := &oauth2.Token{
		RefreshToken: req.RefreshToken,
		Expiry:       time.Now().Add(-1 * time.Hour),
	}
	tokenSource := c.oauthConfig.TokenSource(ctx, expiredToken)
	newToken, err := tokenSource.Token()
	if err != nil {
		logger.Errorf(ctx, "Failed to refresh token: %v", err)
		return nil, code.RefreshTokenErr
	}
	return &login.RefreshTokenResp{
		AccessToken:  newToken.AccessToken,
		RefreshToken: newToken.RefreshToken,
		ExpiresIn:    newToken.Expiry.Unix() - time.Now().Unix(),
		TokenType:    newToken.TokenType,
	}, nil
}

func (c *casdoorLogin) Callback(ctx context.Context, req *login.CallbackReq) (*login.CallbackResp, error) {
	stateKey := fmt.Sprintf("oauth_state:%s", req.State)
	if err := redis.GetClient().Get(ctx, stateKey).Err(); err != nil {
		return nil, code.LoginStateErr
	}
	redis.GetClient().Del(ctx, stateKey)

	stateJSON, err := base64.URLEncoding.DecodeString(req.State)
	if err != nil {
		return nil, code.LoginStateErr
	}
	var stateObj oauthState
	if err := json.Unmarshal(stateJSON, &stateObj); err != nil {
		return nil, code.LoginStateErr
	}

	frontendCallbackURL := stateObj.FrontendCallbackURL
	if frontendCallbackURL == "" {
		frontendCallbackURL = os.Getenv("FRONTEND_BASE_URL")
		if frontendCallbackURL == "" {
			frontendCallbackURL = "http://localhost:32234"
		}
		frontendCallbackURL += "/login/callback"
	}

	token, err := c.oauthConfig.Exchange(ctx, req.Code, oauth2.AccessTypeOffline)
	if err != nil {
		logger.Errorf(ctx, "Token exchange failed: %v", err)
		return nil, code.ExchangeTokenErr
	}

	client := c.oauthConfig.Client(ctx, token)
	resp, err := client.Get(config.Global().OAuth2.UserInfoURL)
	if err != nil {
		logger.Errorf(ctx, "Failed to get user info: %v", err)
		return nil, code.LoginGetUserInfoErr
	}
	defer resp.Body.Close()

	result := &model.UserInfo{}
	if err := json.NewDecoder(resp.Body).Decode(result); err != nil || result.Status != "ok" || result.Data == nil {
		return nil, code.LoginCallbackErr
	}

	return &login.CallbackResp{
		User:                result.Data,
		Token:               token.AccessToken,
		RefreshToken:        token.RefreshToken,
		ExpiresIn:           token.Expiry.Unix() - time.Now().Unix(),
		FrontendCallbackURL: frontendCallbackURL,
	}, nil
}
