package auth

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/scienceol/osdl/internal/config"
	"github.com/scienceol/osdl/pkg/common"
	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/repo"
	"github.com/scienceol/osdl/pkg/repo/bohr"
	"github.com/scienceol/osdl/pkg/repo/casdoor"
	"github.com/scienceol/osdl/pkg/repo/model"
	"github.com/scienceol/osdl/pkg/utils"
	"golang.org/x/oauth2"
)

type AuthType string

const (
	AuthTypeBearer AuthType = "Bearer"
	AuthTypeLab    AuthType = "Lab"
	AuthTypeApi    AuthType = "Api"
)

type AuthFunc func(ctx *gin.Context, authHeader string) *model.UserData

func newAccountClient() repo.Account {
	switch config.Global().Auth.AuthSource {
	case config.AuthBohr:
		return bohr.New()
	case config.AuthCasdoor:
		return casdoor.NewCasClient()
	default:
		panic("unknown auth source: " + string(config.Global().Auth.AuthSource))
	}
}

func ValidateToken(ctx context.Context, tokenType string, token string) (*model.UserData, error) {
	oauthConfig := GetOAuthConfig()
	oauthToken := &oauth2.Token{
		AccessToken: token,
		TokenType:   tokenType,
	}
	client := oauthConfig.Client(ctx, oauthToken)
	conf := config.Global()
	resp, err := client.Get(conf.OAuth2.UserInfoURL)
	if err != nil {
		logger.Errorf(ctx, "Failed to get user info: %v", err)
		return nil, code.InvalidToken
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return nil, code.InvalidToken
	}
	result := &model.UserInfo{}
	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil || result.Status != "ok" || result.Data == nil {
		return nil, code.InvalidToken
	}
	return result.Data, nil
}

func AuthWeb() func(ctx *gin.Context) {
	client := newAccountClient()

	authFuncMap := map[AuthType]AuthFunc{
		AuthTypeLab: getLabUser(client),
		AuthTypeApi: getUserApi(client),
	}

	if config.Global().Auth.AuthSource == config.AuthBohr {
		authFuncMap[AuthTypeBearer] = getBohrUser
	} else {
		authFuncMap[AuthTypeBearer] = getNormalUser
	}

	return Auth(authFuncMap)
}

func AuthLab() func(ctx *gin.Context) {
	client := newAccountClient()
	authFuncMap := map[AuthType]AuthFunc{
		AuthTypeLab: getLabUser(client),
	}
	return Auth(authFuncMap)
}

func Auth(authFuncMap map[AuthType]AuthFunc) func(ctx *gin.Context) {
	return func(ctx *gin.Context) {
		cookie, _ := ctx.Cookie("access_token")
		authHeader := ctx.GetHeader("Authorization")
		queryToken := ctx.Query("access_token")
		authHeader = utils.Or(cookie, queryToken, authHeader)
		if authHeader == "" {
			ctx.JSON(http.StatusUnauthorized, &common.Resp{
				Code:  code.UnLogin,
				Error: &common.Error{Msg: code.UnLogin.String()},
			})
			ctx.Abort()
			return
		}
		tokens := strings.Split(authHeader, " ")
		if len(tokens) != 2 {
			ctx.JSON(http.StatusUnauthorized, &common.Resp{
				Code:  code.LoginFormatErr,
				Error: &common.Error{Msg: code.LoginFormatErr.String()},
			})
			ctx.Abort()
			return
		}
		var userInfo *model.UserData
		if f, ok := authFuncMap[AuthType(tokens[0])]; ok {
			userInfo = f(ctx, tokens[1])
		}
		if userInfo == nil {
			ctx.JSON(http.StatusUnauthorized, &common.Resp{
				Code:  code.LoginFormatErr,
				Error: &common.Error{Msg: code.LoginFormatErr.String()},
			})
			ctx.Abort()
			return
		}
		ctx.Set(USERKEY, userInfo)
		ctx.Next()
	}
}

func getNormalUser(ctx *gin.Context, authHeader string) *model.UserData {
	userInfo, err := ValidateToken(ctx, "Bearer", authHeader)
	if err != nil {
		logger.Errorf(ctx, "Token validation failed: %v", err)
		return nil
	}
	return userInfo
}

func getBohrUser(ctx *gin.Context, authHeader string) *model.UserData {
	user := &utils.Claims{}
	if err := utils.ParseJWTWithPublicKey(authHeader, utils.DefaultPublicKey, user); err != nil {
		logger.Errorf(ctx, "getBohrUser parse jwt token err: %v", err)
		return nil
	}

	if user.Exp <= time.Now().UTC().Unix() {
		return nil
	}

	return &model.UserData{
		ID:    strconv.FormatUint(user.Identity.UserID, 10),
		OrgID: strconv.FormatUint(user.Identity.OrgID, 10),
	}
}

func getUserApi(client repo.Account) AuthFunc {
	return func(ctx *gin.Context, authHeader string) *model.UserData {
		user, err := client.GetLabUserByAccessKey(ctx, authHeader)
		if err != nil {
			logger.Errorf(ctx, "getUserApi parse access key err: %+v", err)
			return nil
		}
		return user
	}
}

func getLabUser(client repo.Account) AuthFunc {
	return func(ctx *gin.Context, authHeader string) *model.UserData {
		baseStr, err := base64.StdEncoding.DecodeString(authHeader)
		if err != nil {
			logger.Errorf(ctx, "getLabUser decode auth header err: %s", err.Error())
			return nil
		}
		keys := strings.Split(string(baseStr), ":")
		if len(keys) != 2 {
			logger.Errorf(ctx, "getLabUser base format err not 2")
			return nil
		}
		userInfo, err := client.GetLabUserInfo(ctx, &model.LabAkSk{
			AccessKey:    keys[0],
			AccessSecret: keys[1],
		})
		if err != nil {
			logger.Errorf(ctx, "getLabUser GetLabUserInfo err: %s", err.Error())
			return nil
		}
		userInfo.AccessKey = keys[0]
		userInfo.AccessSecret = keys[1]
		return userInfo
	}
}

func GetCurrentUser(ctx context.Context) *model.UserData {
	gCtx, ok := ctx.(*gin.Context)
	if !ok {
		return nil
	}
	user, exists := gCtx.Get(USERKEY)
	if !exists {
		return nil
	}
	ud, ok := user.(*model.UserData)
	if !ok {
		return nil
	}
	return ud
}

func IsCH(ctx context.Context) bool {
	gCtx, ok := ctx.(*gin.Context)
	if !ok {
		return true
	}
	return gCtx.GetHeader(LANGUAGE) == "zh-CN"
}
