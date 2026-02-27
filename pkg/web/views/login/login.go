package login

import (
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"

	"github.com/gin-gonic/gin"
	"github.com/scienceol/osdl/pkg/common"
	"github.com/scienceol/osdl/pkg/common/code"
	ls "github.com/scienceol/osdl/pkg/core/login"
	"github.com/scienceol/osdl/pkg/core/login/casdoor"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"golang.org/x/oauth2"
)

type Login struct {
	oauthConfig *oauth2.Config
	lService    ls.Service
}

func NewLogin() *Login {
	return &Login{
		oauthConfig: auth.GetOAuthConfig(),
		lService:    casdoor.NewCasDoorLogin(),
	}
}

func (l *Login) Login(ctx *gin.Context) {
	req := &ls.LoginReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "Invalid login request: %v", err)
	}
	resp, err := l.lService.Login(ctx, req)
	if err != nil {
		common.ReplyErr(ctx, err)
		return
	}
	ctx.Redirect(http.StatusFound, resp.RedirectURL)
}

func (l *Login) Refresh(ctx *gin.Context) {
	req := &ls.RefreshTokenReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "Invalid request format: %v", err)
		common.ReplyErr(ctx, code.RefreshTokenParamErr)
		return
	}
	resp, err := l.lService.Refresh(ctx, req)
	if err != nil {
		common.ReplyErr(ctx, err)
		return
	}
	common.ReplyOk(ctx, resp)
}

func (l *Login) Callback(ctx *gin.Context) {
	req := &ls.CallbackReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "callback param err: %+v", err)
		frontendBaseURL := getDefaultFrontendURL()
		errorURL := fmt.Sprintf("%s/login/callback?error=%s", frontendBaseURL, url.QueryEscape("parameter error"))
		ctx.Redirect(http.StatusFound, errorURL)
		return
	}
	resp, err := l.lService.Callback(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "callback service err: %+v", err)
		frontendBaseURL := getDefaultFrontendURL()
		errorURL := fmt.Sprintf("%s/login/callback?error=%s", frontendBaseURL, url.QueryEscape("login failed"))
		ctx.Redirect(http.StatusFound, errorURL)
		return
	}

	isSecure := ctx.Request.TLS != nil || ctx.GetHeader("X-Forwarded-Proto") == "https"
	ctx.SetCookie("access_token", resp.Token, int(resp.ExpiresIn), "/", "", isSecure, false)
	ctx.SetCookie("refresh_token", resp.RefreshToken, 30*24*60*60, "/", "", isSecure, false)

	if resp.User != nil {
		userInfo := map[string]any{
			"id": resp.User.ID, "name": resp.User.Name,
			"displayName": resp.User.DisplayName, "email": resp.User.Email,
			"avatar": resp.User.Avatar, "type": resp.User.Type,
			"owner": resp.User.Owner, "phone": resp.User.Phone,
		}
		if userJSON, err := json.Marshal(userInfo); err == nil {
			ctx.SetCookie("user_info", base64.URLEncoding.EncodeToString(userJSON), int(resp.ExpiresIn), "/", "", isSecure, false)
		}
	}

	params := url.Values{}
	params.Set("status", "success")
	frontendURL := fmt.Sprintf("%s?%s", resp.FrontendCallbackURL, params.Encode())
	ctx.Redirect(http.StatusFound, frontendURL)
}

func getDefaultFrontendURL() string {
	frontendURL := os.Getenv("FRONTEND_BASE_URL")
	if frontendURL == "" {
		frontendURL = "http://localhost:32234"
	}
	return frontendURL
}
