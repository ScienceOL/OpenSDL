package auth

import (
	// 外部依赖
	"context"
	"encoding/base64"
	"encoding/json"
	"net/http"
	"strconv"
	"strings"
	"time"

	gin "github.com/gin-gonic/gin"
	oauth2 "golang.org/x/oauth2"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	bohr "github.com/scienceol/opensdl/service/pkg/repo/bohr"
	casdoor "github.com/scienceol/opensdl/service/pkg/repo/casdoor"
	model "github.com/scienceol/opensdl/service/pkg/model"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type AuthType string

const (
	AuthTypeBearer AuthType = "Bearer"
	AuthTypeLab    AuthType = "Lab"
	AuthTypeApi    AuthType = "Api"
)

type AuthFunc func(ctx *gin.Context, authHeader string) *model.UserData

// ValidateToken 检查令牌是否有效
func ValidateToken(ctx context.Context, tokenType string, token string) (*model.UserData, error) {
	// 获取OAuth2配置
	oauthConfig := GetOAuthConfig()
	// 创建一个包含传入token的oauth2.Token对象
	oauthToken := &oauth2.Token{
		AccessToken: token,
		TokenType:   tokenType,
	}

	// 使用token构建OAuth2客户端
	client := oauthConfig.Client(ctx, oauthToken)

	// 获取配置中的用户信息URL
	config := config.Global()

	// 获取用户信息 - 如果token有效，这个请求将成功
	resp, err := client.Get(config.OAuth2.UserInfoURL)
	if err != nil {
		logger.Errorf(ctx, "Failed to get user info: %v", err)
		return nil, code.InvalidToken
	}
	logger.Infof(ctx, "Response status: %d", resp.StatusCode)
	defer resp.Body.Close()

	// 如果状态码不是2xx，则认为token无效
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		logger.Errorf(ctx, "Invalid token, status code: %d", resp.StatusCode)
		return nil, code.InvalidToken
	}

	// 解析用户信息
	result := &model.UserInfo{}
	if err := json.NewDecoder(resp.Body).Decode(&result); err != nil ||
		result.Status != "ok" ||
		result.Data == nil {
		logger.Errorf(ctx, "Failed to parse user info: %v", err)
		return nil, err
	}

	// 检查API调用是否成功
	return result.Data, nil
}

func AuthWeb() func(ctx *gin.Context) {
	var client repo.Account
	if config.Global().Auth.AuthSource == config.AuthBohr {
		client = bohr.New()
	} else if config.Global().Auth.AuthSource == config.AuthCasdoor {
		client = casdoor.NewCasClient()
	} else {
		panic("auth type err")
	}

	authFuncMap := map[AuthType]AuthFunc{
		AuthTypeBearer: getBohrUser,
		AuthTypeLab:    getLabUser(client),
		AuthTypeApi:    getUserApi(client),
	}

	return Auth(authFuncMap)
}

func AuthLab() func(ctx *gin.Context) {
	var client repo.Account
	if config.Global().Auth.AuthSource == config.AuthBohr {
		client = bohr.New()
	} else if config.Global().Auth.AuthSource == config.AuthCasdoor {
		client = casdoor.NewCasClient()
	} else {
		panic("auth type err")
	}

	authFuncMap := map[AuthType]AuthFunc{
		AuthTypeLab: getLabUser(client),
	}

	return Auth(authFuncMap)
}

// RequireAuth 中间件函数验证用户是否已登录
func Auth(authFuncMap map[AuthType]AuthFunc) func(ctx *gin.Context) {
	return func(ctx *gin.Context) {
		// 从请求头获取Authorization
		cookie, _ := ctx.Cookie("access_token_v2")
		authHeader := ctx.GetHeader("Authorization")
		queryToken := ctx.Query("access_token_v2")
		authHeader = utils.Or(cookie, queryToken, authHeader)
		if authHeader == "" {
			ctx.JSON(http.StatusUnauthorized, &common.Resp{
				Code: code.UnLogin,
				Error: &common.Error{
					Msg: code.UnLogin.String(),
				},
			})
			ctx.Abort()
			return
		}

		tokens := strings.Split(authHeader, " ")
		if len(tokens) != 2 {
			ctx.JSON(http.StatusUnauthorized,
				&common.Resp{
					Code: code.LoginFormatErr,
					Error: &common.Error{
						Msg: code.LoginFormatErr.String(),
					},
				})
			ctx.Abort()
			return
		}

		var userInfo *model.UserData
		authKey := USERKEY

		f, ok := authFuncMap[AuthType(tokens[0])]
		if ok {
			userInfo = f(ctx, tokens[1])
		}

		if userInfo == nil {
			ctx.JSON(http.StatusUnauthorized,
				&common.Resp{
					Code: code.LoginFormatErr,
					Error: &common.Error{
						Msg: code.LoginFormatErr.String(),
					},
				})
			ctx.Abort()
			return
		}

		// 将用户信息保存到上下文
		ctx.Set(authKey, userInfo)
		ctx.Next()
	}
}

func getBohrUser(ctx *gin.Context, authHeader string) *model.UserData {
	user := &utils.Claims{}
	if err := utils.ParseJWTWithPublicKey(authHeader, utils.DefaultPublicKey, user); err != nil {
		logger.Errorf(ctx, "getBohrUser parse jwt token err")
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

// 实验室用户登录鉴权，通过 sk、sk 获取该实验室创建者用户 id
func getLabUser(client repo.Account) AuthFunc {
	return func(ctx *gin.Context, authHeader string) *model.UserData {
		baseStr, err := base64.StdEncoding.DecodeString(authHeader)
		if err != nil {
			logger.Errorf(ctx, "getLabUser decode auth header err: %s", err.Error())
			return nil
		}

		keys := strings.Split(string(baseStr), ":")
		if len(keys) != 2 {
			logger.Errorf(ctx, "getLabUser base formate err not 2")
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

// cas door 使用的 oauth 登录方式
func getCasdoorUser(ctx *gin.Context, authHeader string) *model.UserData {
	// 检查格式是否为 "Bearer {token}"
	bearerToken := strings.Split(authHeader, " ")
	if len(bearerToken) != 2 || bearerToken[0] != "Bearer" {
		logger.Errorf(ctx, "bearer format err: %s", authHeader)
		return nil
	}

	// 验证令牌
	userInfo, err := ValidateToken(ctx, bearerToken[0], bearerToken[1])
	if err != nil {
		logger.Errorf(ctx, "Token validation failed: %v", err)
		return nil
	}
	return userInfo
}

// GetCurrentUser 从上下文中获取当前用户信息
func GetCurrentUser(ctx context.Context) *model.UserData {
	gCtx, ok := ctx.(*gin.Context)
	if !ok {
		return nil
	}

	user, exists := gCtx.Get(USERKEY)
	if !exists {
		return nil
	}
	return user.(*model.UserData)
}
